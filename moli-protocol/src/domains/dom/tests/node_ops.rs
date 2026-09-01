use super::*;
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandContext, DevToolsDescribeNodeCommand,
    DevToolsDomGeometryCommand, DevToolsDomGeometryOperation, DevToolsDomNodeReference,
    DevToolsErrorKind, DevToolsGetOuterHtmlCommand, DevToolsGetPropertyCommand,
    DevToolsGetTextCommand, DevToolsProtocol, DevToolsQuerySelectorCommand,
    DevToolsScrollIntoViewIfNeededCommand, DevToolsTargetId,
};
use crate::{DevToolsBrowserContextId, DevToolsCommandResult};
use axum::{Router, http::header::CONTENT_TYPE, response::IntoResponse, routing::get};
use tokio::net::TcpListener;

const LOW_BACKEND_OR_FRONTEND_NODE_ID_MISS_FOR_TEST: u32 =
    moli_core::page::RENDERER_BACKEND_NODE_ID_START - 1;

async fn renderer_backend_node_for_live_expression(
    ctx: &mut TestContext,
    evaluate_id: u64,
    describe_id: u64,
    expression: &str,
    depth: i32,
) -> Value {
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
        "params": { "objectId": object_id, "depth": depth }
    }))
    .await;
    take_response_by_id(ctx, describe_id)["result"]["node"].clone()
}

fn loaded_page_mut_for_test(ctx: &mut TestContext) -> &mut moli_core::page::Page {
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_state_mut()
        .runtime_slot
        .loaded_page_mut()
        .expect("loaded page")
}

async fn renderer_frontend_binding_for_test(
    ctx: &mut TestContext,
    frontend_node_id: u32,
) -> moli_core::page::RendererDomFrontendNodeBindingResolution {
    let renderer_inspector_session_id = ctx
        .conn
        .target_renderer_runtime_inspector_session_id_for_session(None);
    let completion = {
        let page = loaded_page_mut_for_test(ctx);
        let pending = page
            .start_document_frontend_node_binding(renderer_inspector_session_id, frontend_node_id)
            .expect("renderer frontend node binding lookup should start");
        pending
            .wait()
            .await
            .expect("renderer frontend node binding lookup should complete")
    };
    let page = loaded_page_mut_for_test(ctx);
    page.finish_document_frontend_node_binding(completion)
        .expect("renderer frontend node binding lookup should finish")
}

async fn append_live_node_without_refreshing_page_snapshot(
    ctx: &mut TestContext,
) -> (u32, moli_core::page::CompletedPageCommand) {
    load_bc(ctx, "BID-A");
    navigate_to_data_html_async(ctx, 1, "<!doctype html><html><body></body></html>").await;
    let backend_node_id = LOW_BACKEND_OR_FRONTEND_NODE_ID_MISS_FOR_TEST;
    let completion = {
        let page = loaded_page_mut_for_test(ctx);
        let mutation = json!({
            "id": 2,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "(() => { const target = document.createElement('button'); target.id = 'fresh-push'; target.setAttribute('data-state', 'live'); const child = document.createElement('span'); child.className = 'fresh-child'; child.textContent = 'fresh text'; target.appendChild(child); document.body.appendChild(target); return 'done'; })()",
                "returnByValue": true
            }
        });
        let pending = page
            .start_runtime_protocol_message(mutation.to_string())
            .expect("runtime mutation should start");
        pending
            .wait()
            .await
            .expect("runtime mutation should complete")
    };
    (backend_node_id, completion)
}

async fn append_live_file_input_without_refreshing_page_snapshot(
    ctx: &mut TestContext,
) -> (u32, moli_core::page::CompletedPageCommand) {
    load_bc(ctx, "BID-A");
    navigate_to_data_html_async(ctx, 1, "<!doctype html><html><body></body></html>").await;
    let backend_node_id = LOW_BACKEND_OR_FRONTEND_NODE_ID_MISS_FOR_TEST;
    let completion = {
        let page = loaded_page_mut_for_test(ctx);
        let mutation = json!({
            "id": 2,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "(() => { const target = document.createElement('input'); target.id = 'fresh-upload'; target.type = 'file'; document.body.appendChild(target); return 'done'; })()",
                "returnByValue": true
            }
        });
        let pending = page
            .start_runtime_protocol_message(mutation.to_string())
            .expect("runtime mutation should start");
        pending
            .wait()
            .await
            .expect("runtime mutation should complete")
    };
    (backend_node_id, completion)
}

fn write_upload_fixture(prefix: &str, bytes: &[u8]) -> (std::path::PathBuf, String) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX_EPOCH")
        .as_nanos();
    let file_path =
        std::env::temp_dir().join(format!("moli-{prefix}-{}-{nanos}.txt", std::process::id()));
    std::fs::write(&file_path, bytes).expect("upload fixture should be writable");
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("upload fixture file name")
        .to_owned();
    (file_path, file_name)
}

#[tokio::test(flavor = "multi_thread")]
async fn get_attributes_returns_interleaved_attribute_array_for_element() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        20,
        "<!doctype html><html><body><div id='target' data-state='ready now' style='color:red' disabled></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 21,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 21)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document node id") as u32;

    ctx.process_async(json!({
        "id": 22,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let node_id = take_query_selector_node_id(&mut ctx, 22) as u32;

    ctx.process_async(json!({
        "id": 23,
        "method": "DOM.getAttributes",
        "params": { "nodeId": node_id }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 23);
    let attrs = interleaved_attributes_to_map(&response["result"]["attributes"]);
    assert_eq!(attrs.get("id").map(String::as_str), Some("target"));
    assert_eq!(
        attrs.get("data-state").map(String::as_str),
        Some("ready now")
    );
    assert_eq!(attrs.get("style").map(String::as_str), Some("color:red"));
    assert_eq!(attrs.get("disabled").map(String::as_str), Some(""));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_attributes_rejects_non_element_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        20,
        "<!doctype html><html><body><p>Paragraph Text</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 21,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 21)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document node id") as u32;

    ctx.process_async(json!({
        "id": 22,
        "method": "DOM.getAttributes",
        "params": { "nodeId": root_id }
    }))
    .await;
    ctx.expect_error(22, -32000, "Node is not an Element");
}

#[tokio::test(flavor = "multi_thread")]
async fn attribute_mutation_commands_emit_chromium_ordered_events_and_update_attributes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='target'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.enable"}))
        .await;
    ctx.expect_result(2, json!({}), None);
    ctx.process_async(json!({"id": 3, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root id");
    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_id = take_query_selector_node_id(&mut ctx, 4);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.setAttributeValue",
        "params": {
            "nodeId": target_id,
            "name": "DATA-STATE",
            "value": "ready"
        }
    }))
    .await;
    assert_eq!(
        ctx.take_all(),
        vec![
            json!({
                "method": "DOM.attributeModified",
                "params": {
                    "nodeId": target_id,
                    "name": "data-state",
                    "value": "ready"
                }
            }),
            json!({ "id": 5, "result": {} }),
        ],
        "Chromium emits the mutation event before the command response"
    );

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.getAttributes",
        "params": { "nodeId": target_id }
    }))
    .await;
    let attributes = interleaved_attributes_to_map(&ctx.take_one()["result"]["attributes"]);
    assert_eq!(
        attributes.get("data-state").map(String::as_str),
        Some("ready")
    );

    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.removeAttribute",
        "params": { "nodeId": target_id, "name": "DATA-STATE" }
    }))
    .await;
    assert_eq!(
        ctx.take_all(),
        vec![
            json!({
                "method": "DOM.attributeRemoved",
                "params": {
                    "nodeId": target_id,
                    "name": "data-state"
                }
            }),
            json!({ "id": 7, "result": {} }),
        ],
        "Chromium emits the removal event before the command response"
    );

    ctx.process_async(json!({
        "id": 8,
        "method": "DOM.getAttributes",
        "params": { "nodeId": target_id }
    }))
    .await;
    let attributes = interleaved_attributes_to_map(&ctx.take_one()["result"]["attributes"]);
    assert!(!attributes.contains_key("data-state"));
}

#[tokio::test(flavor = "multi_thread")]
async fn background_timer_dom_mutation_publishes_without_a_followup_command() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-BACKGROUND-DOM-MUTATION");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='target'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({ "id": 2, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(2, json!({}), None);
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 3)["result"]["root"].clone();
    let target_id = find_cdp_node_by_local_name(&document, "div")
        .expect("target node in complete document snapshot")["nodeId"]
        .as_u64()
        .expect("target frontend node id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "setTimeout(() => document.getElementById('target').setAttribute('data-background', 'published'), 20)",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 4);
    assert_eq!(response["result"]["result"]["type"], json!("number"));

    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "background timer DOM.attributeModified",
        |message| {
            message["method"] == json!("DOM.attributeModified")
                && message["params"]["nodeId"] == json!(target_id)
                && message["params"]["name"] == json!("data-background")
                && message["params"]["value"] == json!("published")
        },
    )
    .await;
}

fn find_cdp_node_by_local_name<'a>(node: &'a Value, local_name: &str) -> Option<&'a Value> {
    if node["localName"] == json!(local_name) {
        return Some(node);
    }
    node["children"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|child| find_cdp_node_by_local_name(child, local_name))
}

fn assert_event_precedes_response(messages: &[Value], method: &str, response_id: u64) {
    let event_index = messages
        .iter()
        .position(|message| message["method"] == json!(method))
        .unwrap_or_else(|| panic!("missing {method} event in {messages:?}"));
    let response_index = messages
        .iter()
        .position(|message| message["id"] == json!(response_id))
        .unwrap_or_else(|| panic!("missing response {response_id} in {messages:?}"));
    assert!(
        event_index < response_index,
        "{method} must precede response {response_id}: {messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_javascript_shallow_dom_snapshot_emits_child_count_before_runtime_response() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><head></head><body></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 2)["result"]["root"].clone();
    let html_node_id = find_cdp_node_by_local_name(&root, "html")
        .and_then(|node| node["nodeId"].as_u64())
        .expect("shallow document snapshot should bind the html element");

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "for (let i = 0; i < 3; i++) document.documentElement.appendChild(document.createElement('aside'))",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeCountUpdated", 3);
    let counts = messages
        .iter()
        .filter(|message| message["method"] == json!("DOM.childNodeCountUpdated"))
        .map(|event| {
            assert_eq!(event["params"]["nodeId"], json!(html_node_id));
            event["params"]["childNodeCount"]
                .as_u64()
                .expect("child count")
        })
        .collect::<Vec<_>>();
    assert_eq!(counts, vec![3, 4, 5]);
}

#[tokio::test(flavor = "multi_thread")]
async fn page_javascript_deep_dom_snapshot_emits_insert_remove_and_character_data_events() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p>before</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 2)["result"]["root"].clone();
    let body_node_id = find_cdp_node_by_local_name(&root, "body")
        .and_then(|node| node["nodeId"].as_u64())
        .expect("deep document snapshot should bind body");
    let text_node_id = find_cdp_node_by_local_name(&root, "p")
        .and_then(|node| node["children"].as_array())
        .and_then(|children| children.first())
        .and_then(|node| node["nodeId"].as_u64())
        .expect("deep document snapshot should bind paragraph text");

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const node = document.createElement('span'); node.id = 'dynamic'; document.body.appendChild(node); return node.localName; })()",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeInserted", 3);
    let inserted = messages
        .iter()
        .find(|message| message["method"] == json!("DOM.childNodeInserted"))
        .expect("insert event");
    assert_eq!(inserted["params"]["parentNodeId"], json!(body_node_id));
    assert_eq!(inserted["params"]["node"]["localName"], json!("span"));
    let inserted_node_id = inserted["params"]["node"]["nodeId"]
        .as_u64()
        .expect("inserted frontend node id");

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.querySelector('#dynamic').remove()",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeRemoved", 4);
    let removed = messages
        .iter()
        .find(|message| message["method"] == json!("DOM.childNodeRemoved"))
        .expect("remove event");
    assert_eq!(removed["params"]["parentNodeId"], json!(body_node_id));
    assert_eq!(removed["params"]["nodeId"], json!(inserted_node_id));

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.querySelector('p').firstChild.data = 'after'",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.characterDataModified", 5);
    let modified = messages
        .iter()
        .find(|message| message["method"] == json!("DOM.characterDataModified"))
        .expect("character data event");
    assert_eq!(modified["params"]["nodeId"], json!(text_node_id));
    assert_eq!(modified["params"]["characterData"], json!("after"));
}

#[tokio::test(flavor = "multi_thread")]
async fn whitespace_visibility_changes_emit_chromium_insert_and_remove_events() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p id='target'>x</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 2)["result"]["root"].clone();
    let paragraph = find_cdp_node_by_local_name(&root, "p").expect("paragraph snapshot");
    let paragraph_id = paragraph["nodeId"].as_u64().expect("paragraph node id");
    let original_text = &paragraph["children"][0];
    let original_text_node_id = original_text["nodeId"].as_u64().expect("text node id");
    let original_backend_node_id = original_text["backendNodeId"]
        .as_u64()
        .expect("text backend node id");

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.__whitespaceTarget = document.querySelector('#target').firstChild; __whitespaceTarget.data = '   '",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeRemoved", 3);
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == json!("DOM.characterDataModified")),
        "a text node becoming whitespace-only leaves the default InspectorDOMAgent tree"
    );
    let removed = messages
        .iter()
        .find(|message| message["method"] == json!("DOM.childNodeRemoved"))
        .expect("text removal event");
    assert_eq!(removed["params"]["parentNodeId"], json!(paragraph_id));
    assert_eq!(removed["params"]["nodeId"], json!(original_text_node_id));

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "__whitespaceTarget.data = 'y'",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeInserted", 4);
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == json!("DOM.characterDataModified")),
        "a formerly hidden whitespace node re-enters the InspectorDOMAgent tree as an insertion"
    );
    let inserted = messages
        .iter()
        .find(|message| message["method"] == json!("DOM.childNodeInserted"))
        .expect("text insertion event");
    assert_eq!(inserted["params"]["parentNodeId"], json!(paragraph_id));
    assert_eq!(inserted["params"]["previousNodeId"], json!(0));
    assert_eq!(inserted["params"]["node"]["nodeValue"], json!("y"));
    assert_eq!(
        inserted["params"]["node"]["backendNodeId"],
        json!(original_backend_node_id),
        "visibility changes preserve the backend node identity"
    );
    assert_ne!(
        inserted["params"]["node"]["nodeId"],
        json!(original_text_node_id),
        "the filtered node receives a fresh frontend id when it becomes visible again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn include_whitespace_all_keeps_text_node_bound_across_whitespace_value_change() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p id='target'>x</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.enable",
        "params": { "includeWhitespace": "all" }
    }))
    .await;
    ctx.expect_result(2, json!({}), None);
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 3)["result"]["root"].clone();
    let text_node_id = find_cdp_node_by_local_name(&root, "p")
        .and_then(|node| node["children"][0]["nodeId"].as_u64())
        .expect("text node id");

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.querySelector('#target').firstChild.data = '   '",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.characterDataModified", 4);
    assert!(messages.iter().any(|message| {
        message["method"] == json!("DOM.characterDataModified")
            && message["params"]["nodeId"] == json!(text_node_id)
            && message["params"]["characterData"] == json!("   ")
    }));
    assert!(!messages.iter().any(|message| {
        matches!(
            message["method"].as_str(),
            Some("DOM.childNodeInserted" | "DOM.childNodeRemoved")
        )
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn whitespace_only_siblings_are_ignored_for_mutations_and_previous_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='left'></div>   <div id='right'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 2)["result"]["root"].clone();
    let body = find_cdp_node_by_local_name(&root, "body").expect("body snapshot");
    let body_node_id = body["nodeId"].as_u64().expect("body node id");
    let left_node_id = body["children"][0]["nodeId"]
        .as_u64()
        .expect("left node id");

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.__extraWhitespace = document.body.insertBefore(document.createTextNode('  '), document.querySelector('#right'))",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_eq!(messages.len(), 1, "unexpected events: {messages:?}");
    assert_eq!(messages[0]["id"], json!(3));
    assert!(
        messages[0].get("result").is_some(),
        "adding a whitespace-only text node must not mutate the default InspectorDOMAgent tree"
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.querySelector('#right').before(Object.assign(document.createElement('span'), { id: 'inserted' }))",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeInserted", 4);
    let inserted = messages
        .iter()
        .find(|message| message["method"] == json!("DOM.childNodeInserted"))
        .expect("span insertion event");
    assert_eq!(inserted["params"]["parentNodeId"], json!(body_node_id));
    assert_eq!(inserted["params"]["previousNodeId"], json!(left_node_id));
    assert_eq!(inserted["params"]["node"]["localName"], json!("span"));
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_mutations_are_projected_from_each_sessions_requested_tree_depth() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-primary");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><head></head><body></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1" }
    }))
    .await;
    let auxiliary_session_id = take_response_by_id(&mut ctx, 2)["result"]["sessionId"]
        .as_str()
        .expect("auxiliary session id")
        .to_owned();
    assert_ne!(auxiliary_session_id, "SID-primary");
    ctx.sent.clear();

    for (id, session_id) in [(3, "SID-primary"), (4, auxiliary_session_id.as_str())] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "DOM.enable"
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(response["sessionId"], json!(session_id));
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 5,
        "sessionId": "SID-primary",
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let primary_root = take_response_by_id(&mut ctx, 5)["result"]["root"].clone();
    let primary_html_id = find_cdp_node_by_local_name(&primary_root, "html")
        .and_then(|node| node["nodeId"].as_u64())
        .expect("primary html node id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "sessionId": auxiliary_session_id,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let auxiliary_root = take_response_by_id(&mut ctx, 6)["result"]["root"].clone();
    let auxiliary_html_id = find_cdp_node_by_local_name(&auxiliary_root, "html")
        .and_then(|node| node["nodeId"].as_u64())
        .expect("auxiliary html node id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7,
        "sessionId": "SID-primary",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.documentElement.appendChild(document.createElement('aside')).localName",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeCountUpdated", 7);
    assert!(messages.iter().any(|message| {
        message["sessionId"] == json!("SID-primary")
            && message["method"] == json!("DOM.childNodeCountUpdated")
            && message["params"]["nodeId"] == json!(primary_html_id)
            && message["params"]["childNodeCount"] == json!(3)
    }));
    assert!(messages.iter().any(|message| {
        message["sessionId"] == json!(auxiliary_session_id)
            && message["method"] == json!("DOM.childNodeInserted")
            && message["params"]["parentNodeId"] == json!(auxiliary_html_id)
            && message["params"]["node"]["localName"] == json!("aside")
    }));
    assert!(!messages.iter().any(|message| {
        message["sessionId"] == json!("SID-primary")
            && message["method"] == json!("DOM.childNodeInserted")
    }));

    ctx.process_async(json!({
        "id": 8,
        "sessionId": auxiliary_session_id,
        "method": "DOM.disable"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 8);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 9,
        "sessionId": "SID-primary",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.documentElement.appendChild(document.createElement('nav')).localName",
            "returnByValue": true
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert!(messages.iter().any(|message| {
        message["sessionId"] == json!("SID-primary")
            && message["method"] == json!("DOM.childNodeCountUpdated")
            && message["params"]["childNodeCount"] == json!(4)
    }));
    assert!(
        !messages
            .iter()
            .any(|message| message["sessionId"] == json!(auxiliary_session_id))
    );
}

fn response_node_id(messages: &[Value], response_id: u64) -> u64 {
    messages
        .iter()
        .find(|message| message["id"] == json!(response_id))
        .and_then(|message| message["result"]["nodeId"].as_u64())
        .unwrap_or_else(|| panic!("missing nodeId response {response_id} in {messages:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn set_attributes_as_text_matches_chromium_parse_replace_and_event_order() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='target' foo='one'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 2)["result"]["root"].clone();
    let root_id = root["nodeId"].as_u64().expect("root node id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_id = take_query_selector_node_id(&mut ctx, 3);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.setAttributesAsText",
        "params": {
            "nodeId": target_id,
            "text": "FOO=\"two\" data-x=\"x\"",
            "name": "foo"
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.attributeModified", 4);
    let modifications = messages
        .iter()
        .filter(|message| message["method"] == json!("DOM.attributeModified"))
        .map(|message| {
            (
                message["params"]["name"].as_str().unwrap().to_owned(),
                message["params"]["value"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        modifications,
        vec![
            ("foo".to_owned(), "two".to_owned()),
            ("data-x".to_owned(), "x".to_owned())
        ]
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getAttributes",
        "params": { "nodeId": target_id }
    }))
    .await;
    let attributes = interleaved_attributes_to_map(&ctx.take_one()["result"]["attributes"]);
    assert_eq!(attributes.get("foo").map(String::as_str), Some("two"));
    assert_eq!(attributes.get("data-x").map(String::as_str), Some("x"));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_node_value_matches_chromium_value_validation_and_event_order() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><p>before</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 2)["result"]["root"].clone();
    let paragraph = find_cdp_node_by_local_name(&root, "p").expect("paragraph snapshot");
    let paragraph_id = paragraph["nodeId"].as_u64().expect("paragraph node id");
    let text_id = paragraph["children"][0]["nodeId"]
        .as_u64()
        .expect("text node id");

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.setNodeValue",
        "params": { "nodeId": text_id, "value": "after" }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.characterDataModified", 3);
    assert!(messages.iter().any(|message| {
        message["method"] == json!("DOM.characterDataModified")
            && message["params"]["nodeId"] == json!(text_id)
            && message["params"]["characterData"] == json!("after")
    }));

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.setNodeValue",
        "params": { "nodeId": text_id, "value": "after" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 4),
        json!({ "id": 4, "result": {} }),
        "a same-value character-data write is still a successful edit"
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.setNodeValue",
        "params": { "nodeId": paragraph_id, "value": "invalid" }
    }))
    .await;
    ctx.expect_error(
        5,
        -32000,
        "Can only set value of text nodes or processing instructions",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_node_name_allows_xml_processing_instruction_target_like_chromium() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(&mut ctx, 1, "<!doctype html><html><body></body></html>").await;

    ctx.process_async(json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const pi = document.createProcessingInstruction('old-target', 'data'); document.insertBefore(pi, document.firstChild); return pi; })()"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 2)["result"]["result"]["objectId"]
        .as_str()
        .expect("processing instruction object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.requestNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let processing_instruction_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .expect("processing instruction frontend node id");

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.setNodeName",
        "params": { "nodeId": processing_instruction_id, "name": "xml" }
    }))
    .await;
    let renamed_node_id = take_response_by_id(&mut ctx, 4)["result"]["nodeId"]
        .as_u64()
        .expect("renamed processing instruction frontend node id");
    assert_ne!(renamed_node_id, processing_instruction_id);

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.firstChild.target",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 5)["result"]["result"]["value"],
        json!("xml")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_node_name_and_move_to_return_the_frontend_id_from_the_insert_event() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='source'><b id='rename' data-copy='same'>text</b></div><div id='target'><i id='anchor'></i></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 2)["result"]["root"].clone();
    let root_id = root["nodeId"].as_u64().expect("root node id");
    let mut selected_node_ids = Vec::new();
    for (id, selector) in [
        (3, "#rename"),
        (4, "#source"),
        (5, "#target"),
        (6, "#anchor"),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "DOM.querySelector",
            "params": { "nodeId": root_id, "selector": selector }
        }))
        .await;
        selected_node_ids.push(take_query_selector_node_id(&mut ctx, id));
    }
    let [rename_id, source_id, target_id, anchor_id] = selected_node_ids.as_slice() else {
        panic!("expected four selected node ids");
    };

    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.setNodeName",
        "params": { "nodeId": *rename_id, "name": "strong" }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeInserted", 7);
    let renamed_id = response_node_id(&messages, 7);
    let inserted = messages
        .iter()
        .find(|message| {
            message["method"] == json!("DOM.childNodeInserted")
                && message["params"]["node"]["localName"] == json!("strong")
        })
        .expect("renamed node insertion");
    assert_eq!(inserted["params"]["node"]["nodeId"], json!(renamed_id));
    assert_ne!(renamed_id, *rename_id);
    let renamed_attributes =
        interleaved_attributes_to_map(&inserted["params"]["node"]["attributes"]);
    assert_eq!(
        renamed_attributes.get("data-copy").map(String::as_str),
        Some("same")
    );

    ctx.process_async(json!({
        "id": 8,
        "method": "DOM.moveTo",
        "params": {
            "nodeId": *source_id,
            "targetNodeId": *target_id,
            "insertBeforeNodeId": *anchor_id
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeRemoved", 8);
    assert_event_precedes_response(&messages, "DOM.childNodeInserted", 8);
    let moved_id = response_node_id(&messages, 8);
    let inserted = messages
        .iter()
        .find(|message| {
            message["method"] == json!("DOM.childNodeInserted")
                && message["params"]["node"]["localName"] == json!("div")
        })
        .expect("moved node insertion");
    assert_eq!(inserted["params"]["node"]["nodeId"], json!(moved_id));
    assert_ne!(moved_id, *source_id);
    assert_eq!(inserted["params"]["previousNodeId"], json!(0));

    ctx.process_async(json!({
        "id": 9,
        "method": "DOM.moveTo",
        "params": {
            "nodeId": moved_id,
            "targetNodeId": *target_id,
            "insertBeforeNodeId": 0
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeRemoved", 9);
    assert_event_precedes_response(&messages, "DOM.childNodeInserted", 9);
    let moved_to_end_id = response_node_id(&messages, 9);
    let inserted = messages
        .iter()
        .find(|message| {
            message["method"] == json!("DOM.childNodeInserted")
                && message["params"]["node"]["localName"] == json!("div")
        })
        .expect("moved-to-end node insertion");
    assert_eq!(inserted["params"]["node"]["nodeId"], json!(moved_to_end_id));
    assert_eq!(inserted["params"]["previousNodeId"], json!(*anchor_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_outer_html_replaces_the_tracked_node_before_responding() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='target'>old</div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let root = take_response_by_id(&mut ctx, 2)["result"]["root"].clone();
    let root_id = root["nodeId"].as_u64().expect("root node id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_id = take_query_selector_node_id(&mut ctx, 3);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.setOuterHTML",
        "params": {
            "nodeId": target_id,
            "outerHTML": "<section id='replacement'><span>new</span></section>"
        }
    }))
    .await;
    let messages = ctx.take_all();
    assert_event_precedes_response(&messages, "DOM.childNodeRemoved", 4);
    assert_event_precedes_response(&messages, "DOM.childNodeInserted", 4);
    assert!(messages.iter().any(|message| {
        message["method"] == json!("DOM.childNodeRemoved")
            && message["params"]["nodeId"] == json!(target_id)
    }));
    assert!(messages.iter().any(|message| {
        message["method"] == json!("DOM.childNodeInserted")
            && message["params"]["node"]["localName"] == json!("section")
    }));

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#replacement" }
    }))
    .await;
    assert_ne!(take_query_selector_node_id(&mut ctx, 5), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn attribute_mutation_commands_preserve_attribute_case_in_xhtml_documents() {
    async fn xhtml_document() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/xhtml+xml")],
            "<html xmlns='http://www.w3.org/1999/xhtml'><body><div id='target'/></body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/document.xhtml", get(xhtml_document)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_url_and_wait_for_load_async(&mut ctx, 1, format!("http://{addr}/document.xhtml"))
        .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_id = take_query_selector_node_id(&mut ctx, 3);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.setAttributeValue",
        "params": {
            "nodeId": target_id,
            "name": "DATA-State",
            "value": "ready"
        }
    }))
    .await;
    assert_eq!(
        ctx.take_all(),
        vec![
            json!({
                "method": "DOM.attributeModified",
                "params": {
                    "nodeId": target_id,
                    "name": "DATA-State",
                    "value": "ready"
                }
            }),
            json!({ "id": 4, "result": {} }),
        ]
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getAttributes",
        "params": { "nodeId": target_id }
    }))
    .await;
    let attributes = interleaved_attributes_to_map(&ctx.take_one()["result"]["attributes"]);
    assert_eq!(
        attributes.get("DATA-State").map(String::as_str),
        Some("ready")
    );
    assert!(!attributes.contains_key("data-state"));
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn attribute_mutation_commands_match_chromium_noop_and_error_semantics() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='target' data-state='ready'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_id = take_query_selector_node_id(&mut ctx, 3);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.setAttributeValue",
        "params": {
            "nodeId": target_id,
            "name": "data-state",
            "value": "ready"
        }
    }))
    .await;
    assert_eq!(ctx.take_all(), vec![json!({ "id": 4, "result": {} })]);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.removeAttribute",
        "params": { "nodeId": target_id, "name": "data-missing" }
    }))
    .await;
    assert_eq!(ctx.take_all(), vec![json!({ "id": 5, "result": {} })]);

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.setAttributeValue",
        "params": {
            "nodeId": root_id,
            "name": "data-state",
            "value": "changed"
        }
    }))
    .await;
    ctx.expect_error(6, -32000, "Node is not an Element");

    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.setAttributeValue",
        "params": {
            "nodeId": 999_999,
            "name": "data-state",
            "value": "changed"
        }
    }))
    .await;
    ctx.expect_error(7, -32000, "Could not find node with given id");

    ctx.process_async(json!({
        "id": 8,
        "method": "DOM.setAttributeValue",
        "params": {
            "nodeId": target_id,
            "name": "bad name",
            "value": "changed"
        }
    }))
    .await;
    ctx.expect_error(
        8,
        -32000,
        "InvalidCharacterError 'bad name' is not a valid attribute name.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attribute_mutation_commands_bypass_page_prototypes_and_notify_mutation_observers() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='target'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_id = take_query_selector_node_id(&mut ctx, 3);

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                globalThis.__attributeMutations = [];
                const target = document.getElementById("target");
                new MutationObserver(records => {
                    for (const record of records) {
                        globalThis.__attributeMutations.push([
                            record.attributeName,
                            record.oldValue,
                            target.getAttribute(record.attributeName)
                        ]);
                    }
                }).observe(target, { attributes: true, attributeOldValue: true });
                Element.prototype.setAttribute = () => { throw new Error("page setAttribute"); };
                Element.prototype.removeAttribute = () => { throw new Error("page removeAttribute"); };
                return true;
            })()"#,
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(ctx.take_one()["result"]["result"]["value"], json!(true));

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.setAttributeValue",
        "params": {
            "nodeId": target_id,
            "name": "data-probe",
            "value": "one"
        }
    }))
    .await;
    ctx.expect_event("DOM.attributeModified", None);
    ctx.expect_result(5, json!({}), None);

    ctx.process_async(json!({
        "id": 6,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const target = document.getElementById('target'); globalThis.__probeAttr = target.getAttributeNode('data-probe'); return JSON.stringify([target.getAttribute('data-probe'), globalThis.__attributeMutations]); })()",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        ctx.take_one()["result"]["result"]["value"],
        json!(r#"["one",[["data-probe",null,"one"]]]"#)
    );

    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.removeAttribute",
        "params": { "nodeId": target_id, "name": "data-probe" }
    }))
    .await;
    ctx.expect_event("DOM.attributeRemoved", None);
    ctx.expect_result(7, json!({}), None);

    ctx.process_async(json!({
        "id": 8,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "JSON.stringify([document.getElementById('target').hasAttribute('data-probe'), globalThis.__probeAttr.ownerElement === null, globalThis.__probeAttr.value, globalThis.__attributeMutations])",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        ctx.take_one()["result"]["result"]["value"],
        json!(r#"[false,true,"one",[["data-probe",null,"one"],["data-probe","one",null]]]"#)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_accepts_all_node_references_and_bypasses_page_prototypes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-FOCUS");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='field'><div id='plain'>text</div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.enable"}))
        .await;
    ctx.expect_result(2, json!({}), None);
    ctx.process_async(json!({"id": 3, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root node id");
    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#field" }
    }))
    .await;
    let field_id = take_query_selector_node_id(&mut ctx, 4);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.describeNode",
        "params": { "nodeId": field_id }
    }))
    .await;
    let backend_id = ctx.take_one()["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("backend node id");
    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.resolveNode",
        "params": { "nodeId": field_id }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["object"]["objectId"]
        .as_str()
        .expect("element object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                globalThis.__focusEvents = [];
                const field = document.getElementById("field");
                for (const name of ["focus", "focusin", "blur", "focusout"]) {
                    field.addEventListener(name, event => __focusEvents.push(event.type));
                }
                HTMLElement.prototype.focus = () => { throw new Error("page focus override"); };
                return true;
            })()"#,
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(ctx.take_one()["result"]["result"]["value"], json!(true));

    for (id, reference) in [
        (8, json!({ "nodeId": field_id })),
        (11, json!({ "backendNodeId": backend_id })),
        (14, json!({ "objectId": object_id })),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "DOM.focus",
            "params": reference
        }))
        .await;
        ctx.expect_result(id, json!({}), None);
        ctx.process_async(json!({
            "id": id + 1,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "JSON.stringify([document.activeElement.id, globalThis.__focusEvents])",
                "returnByValue": true
            }
        }))
        .await;
        assert_eq!(
            ctx.take_one()["result"]["result"]["value"],
            json!(r#"["field",["focus","focusin"]]"#)
        );
        ctx.process_async(json!({
            "id": id + 2,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "document.activeElement.blur(); globalThis.__focusEvents = []; true",
                "returnByValue": true
            }
        }))
        .await;
        assert_eq!(ctx.take_one()["result"]["result"]["value"], json!(true));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_matches_chromium_reference_priority_and_errors() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-FOCUS-ERRORS");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='field'><div id='plain'>text</div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root node id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#field" }
    }))
    .await;
    let field_id = take_query_selector_node_id(&mut ctx, 3);
    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#plain" }
    }))
    .await;
    let plain_id = take_query_selector_node_id(&mut ctx, 4);
    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('plain').firstChild",
            "returnByValue": false
        }
    }))
    .await;
    let text_object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .expect("text object id")
        .to_owned();
    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.requestNode",
        "params": { "objectId": text_object_id }
    }))
    .await;
    let text_id = ctx.take_one()["result"]["nodeId"]
        .as_u64()
        .expect("text node id");

    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.focus",
        "params": {}
    }))
    .await;
    ctx.expect_error(
        7,
        -32000,
        "Either nodeId, backendNodeId or objectId must be specified",
    );
    ctx.process_async(json!({
        "id": 8,
        "method": "DOM.focus",
        "params": { "nodeId": text_id }
    }))
    .await;
    ctx.expect_error(8, -32000, "Node is not an Element");
    ctx.process_async(json!({
        "id": 9,
        "method": "DOM.focus",
        "params": { "nodeId": plain_id }
    }))
    .await;
    ctx.expect_error(9, -32000, "Element is not focusable");
    ctx.process_async(json!({
        "id": 10,
        "method": "DOM.focus",
        "params": { "nodeId": 999_999 }
    }))
    .await;
    ctx.expect_error(10, -32000, "Could not find node with given id");
    ctx.process_async(json!({
        "id": 11,
        "method": "DOM.focus",
        "params": { "backendNodeId": 999_999 }
    }))
    .await;
    ctx.expect_error(11, -32000, "No node found for given backend id");
    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.focus",
        "params": {
            "nodeId": plain_id,
            "backendNodeId": field_id
        }
    }))
    .await;
    ctx.expect_error(12, -32000, "Element is not focusable");
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_object_reference_uses_child_frame_owner_realm() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-CHILD-FOCUS");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe srcdoc=\"<body><input id='child'></body>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;

    ctx.process_async(json!({"id": 3, "method": "Runtime.enable"}))
        .await;
    let _ = take_response_by_id(&mut ctx, 3);
    let child_context_id = child_default_context_id_from_events(&ctx, &child_frame_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": child_context_id,
            "expression": r#"(() => {
                globalThis.__focusEvents = [];
                const child = document.getElementById("child");
                child.addEventListener("focus", event => __focusEvents.push(event.type));
                child.addEventListener("focusin", event => __focusEvents.push(event.type));
                HTMLElement.prototype.focus = () => { throw new Error("child focus override"); };
                return child;
            })()"#,
            "returnByValue": false
        }
    }))
    .await;
    let child_object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .expect("child input object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.focus",
        "params": { "objectId": child_object_id.clone() }
    }))
    .await;
    ctx.expect_result(5, json!({}), None);
    ctx.process_async(json!({
        "id": 6,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": child_object_id,
            "functionDeclaration": "function() { return JSON.stringify([this.ownerDocument.activeElement === this, globalThis.__focusEvents]); }",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 6)["result"]["result"]["value"],
        json!(r#"[true,["focus","focusin"]]"#)
    );
    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.activeElement === document.querySelector('iframe')",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 7)["result"]["result"]["value"],
        json!(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_node_removes_query_selected_live_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        10,
        "<!doctype html><html><body><div class='pw-init' id='one'></div><div class='pw-init' id='two'></div><p id='keep'>ok</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 11,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 11)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document node id") as u32;

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.querySelectorAll",
        "params": {
            "nodeId": root_id,
            "selector": ".pw-init"
        }
    }))
    .await;
    let query_messages = ctx.take_all();
    let query_response_position = query_messages
        .iter()
        .position(|message| message["id"] == json!(12))
        .expect("querySelectorAll response");
    assert!(
        query_messages[..query_response_position]
            .iter()
            .all(|message| message["method"] == json!("DOM.setChildNodes"))
    );
    let node_ids = query_messages[query_response_position]["result"]["nodeIds"]
        .as_array()
        .expect("node ids")
        .iter()
        .filter_map(|value| value.as_u64())
        .map(|value| value as u32)
        .collect::<Vec<_>>();
    assert_eq!(node_ids.len(), 2);

    for (index, node_id) in node_ids.iter().copied().enumerate() {
        let id = 13 + index as u64;
        ctx.process_async(json!({
            "id": id,
            "method": "DOM.removeNode",
            "params": { "nodeId": node_id }
        }))
        .await;
        let remove_messages = ctx.take_all();
        let mutation_position = remove_messages
            .iter()
            .position(|message| {
                message["method"] == json!("DOM.childNodeRemoved")
                    && message["params"]["nodeId"] == json!(node_id)
            })
            .expect("removeNode should publish the removed frontend node");
        let response_position = remove_messages
            .iter()
            .position(|message| message["id"] == json!(id))
            .expect("removeNode response");
        assert!(mutation_position < response_position);
        assert_eq!(remove_messages[response_position]["result"], json!({}));
    }

    ctx.process_async(json!({
        "id": 15,
        "method": "DOM.querySelectorAll",
        "params": {
            "nodeId": root_id,
            "selector": ".pw-init"
        }
    }))
    .await;
    ctx.expect_result(15, json!({ "nodeIds": [] }), None);

    ctx.process_async(json!({
        "id": 16,
        "method": "DOM.querySelector",
        "params": {
            "nodeId": root_id,
            "selector": "body"
        }
    }))
    .await;
    let body_node_id = take_query_selector_node_id(&mut ctx, 16) as u32;

    ctx.process_async(json!({
        "id": 17,
        "method": "DOM.getOuterHTML",
        "params": { "nodeId": body_node_id }
    }))
    .await;
    let outer_html_response = take_response_by_id(&mut ctx, 17);
    let outer_html = outer_html_response["result"]["outerHTML"]
        .as_str()
        .expect("outerHTML");
    assert!(
        !outer_html.contains("pw-init"),
        "removed nodes should disappear from live DOM: {outer_html}"
    );
    assert!(
        outer_html.contains("keep"),
        "non-target nodes should remain after cleanup: {outer_html}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_remove_node_removes_query_selected_live_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        10,
        "<!doctype html><html><body><div class='pw-init' id='one'></div><div class='pw-init' id='two'></div><p id='keep'>ok</p></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 11,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 11)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document node id") as u32;

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.querySelectorAll",
        "params": {
            "nodeId": root_id,
            "selector": ".pw-init"
        }
    }))
    .await;
    let query_messages = ctx.take_all();
    let query_response_position = query_messages
        .iter()
        .position(|message| message["id"] == json!(12))
        .expect("querySelectorAll response");
    assert!(
        query_messages[..query_response_position]
            .iter()
            .all(|message| message["method"] == json!("DOM.setChildNodes"))
    );
    let node_ids = query_messages[query_response_position]["result"]["nodeIds"]
        .as_array()
        .expect("node ids")
        .iter()
        .filter_map(|value| value.as_u64())
        .map(|value| value as u32)
        .collect::<Vec<_>>();
    assert_eq!(node_ids.len(), 2);

    for (index, node_id) in node_ids.iter().copied().enumerate() {
        let id = 13 + index as u64;
        ctx.process_async(json!({
            "id": id,
            "method": "DOM.removeNode",
            "params": { "nodeId": node_id }
        }))
        .await;
        let remove_messages = ctx.take_all();
        let mutation_position = remove_messages
            .iter()
            .position(|message| {
                message["method"] == json!("DOM.childNodeRemoved")
                    && message["params"]["nodeId"] == json!(node_id)
            })
            .expect("removeNode should publish the removed frontend node");
        let response_position = remove_messages
            .iter()
            .position(|message| message["id"] == json!(id))
            .expect("removeNode response");
        assert!(mutation_position < response_position);
        assert_eq!(remove_messages[response_position]["result"], json!({}));
    }

    ctx.process_async(json!({
        "id": 15,
        "method": "DOM.querySelectorAll",
        "params": {
            "nodeId": root_id,
            "selector": ".pw-init"
        }
    }))
    .await;
    ctx.expect_result(15, json!({ "nodeIds": [] }), None);

    ctx.process_async(json!({
        "id": 16,
        "method": "DOM.querySelector",
        "params": {
            "nodeId": root_id,
            "selector": "body"
        }
    }))
    .await;
    let body_node_id = take_query_selector_node_id(&mut ctx, 16) as u32;

    ctx.process_async(json!({
        "id": 17,
        "method": "DOM.getOuterHTML",
        "params": { "nodeId": body_node_id }
    }))
    .await;
    let outer_html_response = take_response_by_id(&mut ctx, 17);
    let outer_html = outer_html_response["result"]["outerHTML"]
        .as_str()
        .expect("outerHTML");
    assert!(
        !outer_html.contains("pw-init"),
        "removed nodes should disappear from live DOM: {outer_html}"
    );
    assert!(
        outer_html.contains("keep"),
        "non-target nodes should remain after cleanup: {outer_html}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_file_input_files_updates_file_input_for_frontend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='upload' type='file'></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let document = ctx.take_one();
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#upload" }
    }))
    .await;
    let input_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .expect("input node id");

    let upload_bytes = b"hello upload";
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX_EPOCH")
        .as_nanos();
    let file_path = std::env::temp_dir().join(format!(
        "moli-cdp-upload-{}-{nanos}.txt",
        std::process::id()
    ));
    std::fs::write(&file_path, upload_bytes).expect("upload fixture should be writable");
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("upload fixture file name")
        .to_owned();

    let raw = json!({
        "id": 4,
        "method": "DOM.setFileInputFiles",
        "params": {
            "nodeId": input_node_id,
            "files": [file_path.to_string_lossy()]
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("nodeId DOM.setFileInputFiles should start a renderer command");
    assert_eq!(pending.kind_name(), "DOM");
    let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;
    let _ = std::fs::remove_file(&file_path);
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(4))
        .unwrap_or_else(|| panic!("pending DOM.setFileInputFiles should respond: {messages:?}"));
    assert_eq!(response["result"], json!({}));

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const files = document.querySelector('#upload').files; return `${files.length}:${files[0].name}:${files[0].size}`; })()",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 5)["result"]["result"]["value"],
        json!(format!("1:{file_name}:{}", upload_bytes.len()))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_file_input_files_object_id_reads_live_document_after_document_open_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='old-upload'></div></body></html>",
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
                document.write("<!doctype html><html><body><input id='fresh-upload' type='file'></body></html>");
                document.close();
                return document.querySelector('#fresh-upload');
            })()"#
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 12)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("replacement Runtime.evaluate should return objectId"))
        .to_owned();

    let upload_bytes = b"fresh object upload";
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX_EPOCH")
        .as_nanos();
    let file_path = std::env::temp_dir().join(format!(
        "moli-cdp-object-upload-{}-{nanos}.txt",
        std::process::id()
    ));
    std::fs::write(&file_path, upload_bytes).expect("upload fixture should be writable");
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("upload fixture file name")
        .to_owned();

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.setFileInputFiles",
        "params": {
            "objectId": object_id,
            "files": [file_path.to_string_lossy()]
        }
    }))
    .await;
    let _ = std::fs::remove_file(&file_path);
    ctx.expect_result(13, json!({}), None);

    ctx.process_async(json!({
        "id": 14,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const files = document.querySelector('#fresh-upload').files; return `${files.length}:${files[0].name}:${files[0].size}`; })()",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 14)["result"]["result"]["value"],
        json!(format!("1:{file_name}:{}", upload_bytes.len()))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_file_input_files_object_id_rejects_non_file_input() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='not-upload'></div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#not-upload')" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 11)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("Runtime.evaluate should return objectId"))
        .to_owned();

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX_EPOCH")
        .as_nanos();
    let file_path = std::env::temp_dir().join(format!(
        "moli-cdp-object-reject-upload-{}-{nanos}.txt",
        std::process::id()
    ));
    std::fs::write(&file_path, b"not an upload target").expect("upload fixture should be writable");

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.setFileInputFiles",
        "params": {
            "objectId": object_id,
            "files": [file_path.to_string_lossy()]
        }
    }))
    .await;
    let _ = std::fs::remove_file(&file_path);
    ctx.expect_error(12, -32000, "UnableToSetFileInput");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_file_input_files_validates_node_before_reading_files() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='upload' type='file'></body></html>",
    )
    .await;

    let missing_file = std::env::temp_dir().join(format!(
        "moli-missing-upload-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after UNIX_EPOCH")
            .as_nanos()
    ));
    assert!(
        !missing_file.exists(),
        "test path should not exist before stale-node upload"
    );

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.setFileInputFiles",
        "params": {
            "nodeId": 999_999,
            "files": [missing_file.to_string_lossy()]
        }
    }))
    .await;
    ctx.expect_error(2, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn file_chooser_backend_node_id_resolves_detached_source_after_document_open() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='picker' type='file' multiple></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.setInterceptFileChooserDialog",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(2, json!({}), None);
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
                document.getElementById('picker').click();
                document.open();
                document.write("<!doctype html><html><body><input id='fresh' type='file'></body></html>");
                document.close();
                return document.querySelector('#fresh').id;
            })()"#,
            "returnByValue": true
        }
    }))
    .await;
    let chooser_position = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.fileChooserOpened"))
        .unwrap_or_else(|| panic!("Page.fileChooserOpened should be emitted: {:?}", ctx.sent));
    let response_position = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(3))
        .expect("Runtime.evaluate response");
    assert!(
        chooser_position < response_position,
        "Chromium emits fileChooserOpened synchronously before the invoking script response: {:?}",
        ctx.sent
    );
    let evaluated = take_response_by_id(&mut ctx, 3);
    assert_eq!(evaluated["result"]["result"]["value"], json!("fresh"));

    let file_chooser = ctx
        .sent
        .iter()
        .find(|message| message["method"] == json!("Page.fileChooserOpened"))
        .cloned()
        .unwrap_or_else(|| panic!("Page.fileChooserOpened should be emitted: {:?}", ctx.sent));
    assert_eq!(
        file_chooser["params"]["mode"],
        json!("selectMultiple"),
        "old input had multiple=true before document replacement: {file_chooser:?}"
    );
    let backend_node_id = file_chooser["params"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("file chooser event should include u32 backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "file chooser event should use renderer backend id namespace: {file_chooser:?}"
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": backend_node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 4);
    assert_eq!(
        resolved["result"]["object"]["subtype"],
        json!("node"),
        "Chromium keeps the event-exposed detached input resolvable after document.open: {resolved:?}"
    );
    assert!(
        resolved["result"]["object"]["objectId"].as_str().is_some(),
        "detached file input should resolve to a runtime object: {resolved:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_flattened_document_returns_flat_nodes_with_parent_ids() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='parent'><span id='child'>ok</span></div></body></html>",
    )
    .await;

    ctx.process_async(json!({ "id": 20, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(20, json!({}), None);

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getFlattenedDocument",
        "params": { "depth": -1 }
    }))
    .await;

    let response = ctx.take_one();
    let nodes = response["result"]["nodes"]
        .as_array()
        .expect("flattened nodes");
    assert_eq!(
        nodes.first().and_then(|node| node["nodeName"].as_str()),
        Some("#document")
    );
    let body = nodes
        .iter()
        .find(|node| node["nodeName"] == json!("BODY"))
        .expect("body node");
    let parent = flat_node_by_attribute(nodes, "id", "parent");
    let child = flat_node_by_attribute(nodes, "id", "child");

    assert!(parent.get("children").is_none());
    assert!(child.get("children").is_none());
    assert_eq!(parent["parentId"], body["nodeId"]);
    assert_eq!(child["parentId"], parent["nodeId"]);
    let child_backend_node_id = child["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("child backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(child_backend_node_id),
        "DOM.getFlattenedDocument should assign renderer backend ids: {child}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn push_nodes_by_backend_ids_to_frontend_maps_known_nodes_and_zero_for_missing() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><main id='main'><button id='target'>go</button></main></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let document = ctx.take_one();
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root id");
    let root_backend_node_id = document["result"]["root"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("root backend node id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .expect("target id");
    ctx.process_async(json!({
        "id": 31,
        "method": "DOM.describeNode",
        "params": { "nodeId": target_node_id }
    }))
    .await;
    let target_backend_node_id =
        take_response_by_id(&mut ctx, 31)["result"]["node"]["backendNodeId"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .expect("target backend node id");

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.pushNodesByBackendIdsToFrontend",
        "params": { "backendNodeIds": [target_backend_node_id, 999999, root_backend_node_id] }
    }))
    .await;

    ctx.expect_result(4, json!({ "nodeIds": [target_node_id, 0, root_id] }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_whitespace_nodes_keep_backend_identity_without_default_frontend_binding() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-whitespace-default");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body>\n  <main id='target'>value</main>\n</body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1" }
    }))
    .await;
    let all_session_id = take_response_by_id(&mut ctx, 2)["result"]["sessionId"]
        .as_str()
        .expect("auxiliary session id")
        .to_owned();
    ctx.sent.clear();

    for (id, session_id, params) in [
        (3, "SID-whitespace-default", json!({})),
        (
            4,
            all_session_id.as_str(),
            json!({ "includeWhitespace": "all" }),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "DOM.enable",
            "params": params,
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(response["result"], json!({}));
        assert_eq!(response["sessionId"], json!(session_id));
        ctx.sent.clear();
    }

    for (id, session_id) in [(5, "SID-whitespace-default"), (6, all_session_id.as_str())] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "DOM.getDocument",
            "params": { "depth": -1 },
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(response["sessionId"], json!(session_id));
        assert_eq!(response["result"]["root"]["nodeName"], json!("#document"));
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 7,
        "sessionId": all_session_id,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.body.firstChild" }
    }))
    .await;
    let all_object_id = take_response_by_id(&mut ctx, 7)["result"]["result"]["objectId"]
        .as_str()
        .expect("all-mode whitespace object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 8,
        "sessionId": all_session_id,
        "method": "DOM.describeNode",
        "params": { "objectId": all_object_id, "depth": 0 }
    }))
    .await;
    let all_node = take_response_by_id(&mut ctx, 8)["result"]["node"].clone();
    let all_node_id = all_node["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("all-mode whitespace frontend node id");
    let backend_node_id = all_node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("whitespace backend node id");
    assert!(all_node_id > 0);
    assert_eq!(all_node["nodeName"], json!("#text"));
    assert!(
        all_node["nodeValue"]
            .as_str()
            .is_some_and(|value| value.trim().is_empty())
    );
    ctx.sent.clear();

    for (id, session_id, expected_node_id) in [
        (9, "SID-whitespace-default", 0),
        (10, all_session_id.as_str(), all_node_id),
    ] {
        ctx.process_async(json!({
            "id": id,
            "sessionId": session_id,
            "method": "DOM.pushNodesByBackendIdsToFrontend",
            "params": { "backendNodeIds": [backend_node_id] }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(response["sessionId"], json!(session_id));
        assert_eq!(
            response["result"]["nodeIds"],
            json!([expected_node_id]),
            "pushNodes must preserve each session's whitespace projection: {response:?}"
        );
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 11,
        "sessionId": "SID-whitespace-default",
        "method": "Runtime.evaluate",
        "params": { "expression": "document.body.firstChild" }
    }))
    .await;
    let default_object_id = take_response_by_id(&mut ctx, 11)["result"]["result"]["objectId"]
        .as_str()
        .expect("default-mode whitespace object id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 12,
        "sessionId": "SID-whitespace-default",
        "method": "DOM.requestNode",
        "params": { "objectId": default_object_id }
    }))
    .await;
    let requested = take_response_by_id(&mut ctx, 12);
    assert_eq!(requested["result"]["nodeId"], json!(0));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 13,
        "sessionId": "SID-whitespace-default",
        "method": "DOM.describeNode",
        "params": { "objectId": default_object_id, "depth": 0 }
    }))
    .await;
    let default_node = take_response_by_id(&mut ctx, 13)["result"]["node"].clone();
    assert_eq!(default_node["nodeId"], json!(0));
    assert_eq!(default_node["backendNodeId"], json!(backend_node_id));
    assert_eq!(default_node["nodeName"], json!("#text"));
}

#[tokio::test(flavor = "multi_thread")]
async fn push_nodes_by_backend_ids_to_frontend_requires_requested_document() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><button>go</button></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.pushNodesByBackendIdsToFrontend",
        "params": { "backendNodeIds": [1] }
    }))
    .await;

    ctx.expect_error(4, -32000, "Document needs to be requested first");
}

#[tokio::test(flavor = "multi_thread")]
async fn push_nodes_by_backend_ids_to_frontend_supports_renderer_backend_node_ids() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><main id='root'><button id='target'>go</button></main></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let described = renderer_backend_node_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#root')",
        1,
    )
    .await;
    let root_node_id = described["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live root should return frontend nodeId");
    let root_backend_node_id = described["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live root should return backendNodeId");
    let child_node_id = described["children"][0]["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live child should return frontend nodeId");
    let child_backend_node_id = described["children"][0]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live child should return backendNodeId");
    for backend_node_id in [root_backend_node_id, child_backend_node_id] {
        assert!(
            moli_core::page::is_renderer_backend_node_id(backend_node_id),
            "live describeNode should use renderer backend id namespace: {described}"
        );
    }
    let missing_renderer_backend_node_id = moli_core::page::RENDERER_BACKEND_NODE_ID_START + 50_000;
    ctx.process_async(json!({
        "id": 121,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let fresh_document = take_response_by_id(&mut ctx, 121);
    let fresh_nodes = &fresh_document["result"]["root"];
    let fresh_html = child_element_by_node_name(fresh_nodes, "HTML");
    let fresh_body = child_element_by_node_name(fresh_html, "BODY");
    let fresh_root = child_element_by_node_name(fresh_body, "MAIN");
    let fresh_child = child_element_by_node_name(fresh_root, "BUTTON");
    let fresh_root_node_id = fresh_root["nodeId"].as_u64().expect("fresh root node id");
    let fresh_child_node_id = fresh_child["nodeId"].as_u64().expect("fresh child node id");
    assert_ne!(fresh_root_node_id, u64::from(root_node_id));
    assert_ne!(fresh_child_node_id, u64::from(child_node_id));

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.pushNodesByBackendIdsToFrontend",
        "params": {
            "backendNodeIds": [
                child_backend_node_id,
                missing_renderer_backend_node_id,
                root_backend_node_id
            ]
        }
    }))
    .await;
    ctx.expect_result(
        13,
        json!({ "nodeIds": [fresh_child_node_id, 0, fresh_root_node_id] }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn push_nodes_renderer_backend_id_is_scoped_to_document_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><button id='old'>old</button></body></html>",
    )
    .await;
    ctx.process_async(json!({ "id": 9, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(9, json!({}), None);

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let described = renderer_backend_node_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#old')",
        0,
    )
    .await;
    let backend_node_id = described["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live old node should return backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "old node should use renderer backend id namespace: {described}"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.open(); document.write(\"<!doctype html><html><body><button id='fresh'>fresh</button></body></html>\"); document.close(); document.querySelector('#fresh')"
        }
    }))
    .await;
    let fresh = take_response_by_id(&mut ctx, 13);
    assert!(
        fresh["result"]["result"]["objectId"].as_str().is_some(),
        "replacement element should produce a fresh object: {fresh}"
    );
    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "document.open DOMContentLoaded binding refresh",
        |message| message["method"] == json!("DOM.documentUpdated"),
    )
    .await;
    ctx.process_async(json!({"id": 131, "method": "DOM.getDocument"}))
        .await;
    let _ = take_response_by_id(&mut ctx, 131);

    ctx.process_async(json!({
        "id": 14,
        "method": "DOM.pushNodesByBackendIdsToFrontend",
        "params": { "backendNodeIds": [backend_node_id] }
    }))
    .await;
    ctx.expect_result(14, json!({ "nodeIds": [0] }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_attributes_and_push_backend_ids_use_pending_renderer_dispatch() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><button id='target' data-state='ready'>go</button></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let document = take_response_by_id(&mut ctx, 2);
    let root_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root id");
    let root_backend_node_id = document["result"]["root"]["backendNodeId"]
        .as_u64()
        .expect("root backend id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .expect("target id");

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getAttributes",
        "params": { "nodeId": target_node_id }
    }))
    .await;
    let get_attributes = take_response_by_id(&mut ctx, 4);
    let attrs = interleaved_attributes_to_map(&get_attributes["result"]["attributes"]);
    assert_eq!(attrs.get("id").map(String::as_str), Some("target"));
    assert_eq!(attrs.get("data-state").map(String::as_str), Some("ready"));
    ctx.process_async(json!({
        "id": 31,
        "method": "DOM.describeNode",
        "params": { "nodeId": target_node_id }
    }))
    .await;
    let target_backend_node_id =
        take_response_by_id(&mut ctx, 31)["result"]["node"]["backendNodeId"]
            .as_u64()
            .expect("target backend id");

    let push_backend_ids_raw = json!({
        "id": 5,
        "method": "DOM.pushNodesByBackendIdsToFrontend",
        "params": { "backendNodeIds": [target_backend_node_id, 999999, root_backend_node_id] }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(push_pending) =
        ctx.conn.start_command_dispatch(&push_backend_ids_raw)
    else {
        panic!("DOM.pushNodesByBackendIdsToFrontend should start a renderer command")
    };
    let push_backend_ids = complete_pending_command_task_for_test(&mut ctx, *push_pending).await;
    let pushed = push_backend_ids
        .iter()
        .find(|message| message["id"] == json!(5))
        .unwrap_or_else(|| panic!("pushNodes should respond: {push_backend_ids:?}"));
    let pushed_node_ids = pushed["result"]["nodeIds"]
        .as_array()
        .expect("pushed frontend node ids");
    assert_eq!(pushed_node_ids.len(), 3);
    assert_eq!(pushed_node_ids[1], json!(0));
    assert!(pushed_node_ids[0].as_u64().is_some_and(|id| id > 0));
    assert!(pushed_node_ids[2].as_u64().is_some_and(|id| id > 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn push_nodes_by_renderer_backend_ids_binds_fresh_live_dom() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(&mut ctx, 1, "<!doctype html><html><body></body></html>").await;
    ctx.process_async(json!({"id": 902, "method": "DOM.getDocument"}))
        .await;
    let _ = take_response_by_id(&mut ctx, 902);
    let mutation_completion = {
        let page = loaded_page_mut_for_test(&mut ctx);
        let mutation = json!({
            "id": 2,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "(() => { const target = document.createElement('button'); target.id = 'fresh-push'; target.setAttribute('data-state', 'live'); const child = document.createElement('span'); child.className = 'fresh-child'; child.textContent = 'fresh text'; target.appendChild(child); document.body.appendChild(target); return 'done'; })()",
                "returnByValue": true
            }
        });
        let pending = page
            .start_runtime_protocol_message(mutation.to_string())
            .expect("runtime mutation should start");
        pending
            .wait()
            .await
            .expect("runtime mutation should complete")
    };
    let described = renderer_backend_node_for_live_expression(
        &mut ctx,
        903,
        904,
        "document.querySelector('#fresh-push')",
        0,
    )
    .await;
    let backend_node_id = described["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("fresh node renderer backend id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "fresh node should use renderer backend id: {described}"
    );

    let raw = json!({
        "id": 3,
        "method": "DOM.pushNodesByBackendIdsToFrontend",
        "params": { "backendNodeIds": [backend_node_id] }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.pushNodesByBackendIdsToFrontend should query renderer backend binding");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    let pushed_response = messages
        .iter()
        .find(|message| message["id"] == json!(3))
        .unwrap_or_else(|| panic!("pushNodes should respond: {messages:?}"));
    let pushed_node_id = pushed_response["result"]["nodeIds"][0]
        .as_u64()
        .expect("pushNodes should return frontend node id");
    assert_ne!(pushed_node_id, 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getAttributes",
        "params": { "nodeId": pushed_node_id }
    }))
    .await;
    let get_attributes = take_response_by_id(&mut ctx, 4);
    assert!(
        get_attributes["result"]["attributes"].is_array(),
        "DOM.getAttributes should return attributes after pushNodes binding: {get_attributes}"
    );
    let attrs = interleaved_attributes_to_map(&get_attributes["result"]["attributes"]);
    assert_eq!(attrs.get("id").map(String::as_str), Some("fresh-push"));
    assert_eq!(attrs.get("data-state").map(String::as_str), Some("live"));

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_attributes_low_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.getAttributes",
        "params": { "nodeId": node_id }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.getAttributes should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_attributes_unknown_frontend_node_id_misses_after_renderer_lookup() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='target' data-state='protocol-binding'></body></html>",
    )
    .await;

    let fake_frontend_node_id = 999_999;
    let renderer_binding =
        renderer_frontend_binding_for_test(&mut ctx, fake_frontend_node_id).await;
    assert_eq!(
        renderer_binding,
        moli_core::page::RendererDomFrontendNodeBindingResolution::NotFound,
        "renderer DOM agent must not know the unknown frontend node id"
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getAttributes",
        "params": { "nodeId": fake_frontend_node_id }
    }))
    .await;
    ctx.expect_error(4, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn text_and_property_unknown_frontend_node_ids_miss_after_renderer_lookup() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='target' data-state='protocol-binding'>real text</section></body></html>",
    )
    .await;

    let fake_text_frontend_node_id = 999_998;
    let fake_property_frontend_node_id = 999_997;
    for frontend_node_id in [fake_text_frontend_node_id, fake_property_frontend_node_id] {
        let renderer_binding = renderer_frontend_binding_for_test(&mut ctx, frontend_node_id).await;
        assert_eq!(
            renderer_binding,
            moli_core::page::RendererDomFrontendNodeBindingResolution::NotFound,
            "renderer DOM agent must not know the unknown frontend node id"
        );
    }

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::Cdp,
        session_id: None,
        target_id: None,
        browser_context_id: None,
    };

    let (text_result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetText(DevToolsGetTextCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::FrontendNodeId(fake_text_frontend_node_id),
        }))
        .await
        .into_parts();
    let text_error =
        text_result.expect_err("fake text frontend id should not use protocol binding");
    assert_eq!(text_error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(text_error.message, "Could not find node with given id");

    let (property_result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetProperty(DevToolsGetPropertyCommand {
            context,
            reference: DevToolsDomNodeReference::FrontendNodeId(fake_property_frontend_node_id),
            name: "id".to_owned(),
        }))
        .await
        .into_parts();
    let property_error =
        property_result.expect_err("fake property frontend id should not use protocol binding");
    assert_eq!(property_error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(property_error.message, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn node_read_unknown_frontend_node_ids_miss_after_renderer_lookup() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='target' data-state='protocol-binding'>real text</section></body></html>",
    )
    .await;

    let fake_describe_frontend_node_id = 999_996;
    let fake_outer_html_frontend_node_id = 999_995;
    let fake_geometry_frontend_node_id = 999_994;
    let fake_scroll_frontend_node_id = 999_993;
    for frontend_node_id in [
        fake_describe_frontend_node_id,
        fake_outer_html_frontend_node_id,
        fake_geometry_frontend_node_id,
        fake_scroll_frontend_node_id,
    ] {
        let renderer_binding = renderer_frontend_binding_for_test(&mut ctx, frontend_node_id).await;
        assert_eq!(
            renderer_binding,
            moli_core::page::RendererDomFrontendNodeBindingResolution::NotFound,
            "renderer DOM agent must not know the unknown frontend node id"
        );
    }

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: None,
        target_id: None,
        browser_context_id: None,
    };

    let commands = [
        DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(
                fake_describe_frontend_node_id,
            )),
            depth: 0,
            pierce: false,
        }),
        DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(
                fake_outer_html_frontend_node_id,
            )),
            include_shadow_dom: false,
        }),
        DevToolsCommand::DomGeometry(DevToolsDomGeometryCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::FrontendNodeId(fake_geometry_frontend_node_id),
            operation: DevToolsDomGeometryOperation::GetBoxModel,
        }),
        DevToolsCommand::ScrollIntoViewIfNeeded(DevToolsScrollIntoViewIfNeededCommand {
            context,
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(
                fake_scroll_frontend_node_id,
            )),
            rect: None,
        }),
    ];

    for command in commands {
        let (result, _) = ctx
            .conn
            .execute_devtools_command(command)
            .await
            .into_parts();
        let error = result.expect_err("fake frontend id should not use protocol binding");
        assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
        assert_eq!(error.message, "Could not find node with given id");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn set_file_input_files_unknown_frontend_node_id_misses_before_file_read() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='target' type='file'></body></html>",
    )
    .await;

    let fake_frontend_node_id = 999_992;
    let renderer_binding =
        renderer_frontend_binding_for_test(&mut ctx, fake_frontend_node_id).await;
    assert_eq!(
        renderer_binding,
        moli_core::page::RendererDomFrontendNodeBindingResolution::NotFound,
        "renderer DOM agent must not know the unknown frontend node id"
    );

    let missing_file = std::env::temp_dir().join(format!(
        "moli-missing-upload-{}-{fake_frontend_node_id}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&missing_file);
    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.setFileInputFiles",
        "params": {
            "nodeId": fake_frontend_node_id,
            "files": [missing_file.to_string_lossy()]
        }
    }))
    .await;
    ctx.expect_error(4, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_file_input_files_low_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (node_id, mutation_completion) =
        append_live_file_input_without_refreshing_page_snapshot(&mut ctx).await;
    let (file_path, _file_name) = write_upload_fixture("cdp-stale-node-upload", b"fresh upload");

    let raw = json!({
        "id": 3,
        "method": "DOM.setFileInputFiles",
        "params": {
            "nodeId": node_id,
            "files": [file_path.to_string_lossy()]
        }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        let _ = std::fs::remove_file(&file_path);
        panic!("DOM.setFileInputFiles should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    let _ = std::fs::remove_file(&file_path);
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_file_input_files_low_backend_id_misses_without_backend_binding() {
    let mut ctx = TestContext::new();
    let (backend_node_id, mutation_completion) =
        append_live_file_input_without_refreshing_page_snapshot(&mut ctx).await;
    let (file_path, _file_name) =
        write_upload_fixture("cdp-stale-backend-upload", b"fresh backend upload");

    let raw = json!({
        "id": 3,
        "method": "DOM.setFileInputFiles",
        "params": {
            "backendNodeId": backend_node_id,
            "files": [file_path.to_string_lossy()]
        }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        let _ = std::fs::remove_file(&file_path);
        panic!("DOM.setFileInputFiles should preflight renderer backend binding");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    let _ = std::fs::remove_file(&file_path);
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_low_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.resolveNode",
        "params": { "nodeId": node_id }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.resolveNode should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_low_backend_id_misses_without_backend_binding() {
    let mut ctx = TestContext::new();
    let (backend_node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.resolveNode",
        "params": { "backendNodeId": backend_node_id }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.resolveNode should query renderer backend binding");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_low_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.describeNode",
        "params": { "nodeId": node_id, "depth": 0 }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.describeNode should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outer_html_low_backend_id_misses_without_backend_binding() {
    let mut ctx = TestContext::new();
    let (backend_node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.getOuterHTML",
        "params": { "backendNodeId": backend_node_id }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.getOuterHTML should query renderer backend binding");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_box_model_low_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.getBoxModel",
        "params": { "nodeId": node_id }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.getBoxModel should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_content_quads_low_backend_id_misses_without_backend_binding() {
    let mut ctx = TestContext::new();
    let (backend_node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.getContentQuads",
        "params": { "backendNodeId": backend_node_id }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.getContentQuads should query renderer backend binding");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_into_view_low_backend_id_misses_without_backend_binding() {
    let mut ctx = TestContext::new();
    let (backend_node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "backendNodeId": backend_node_id }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.scrollIntoViewIfNeeded should query renderer backend binding");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_low_root_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (root_node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_node_id, "selector": ".fresh-child" }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.querySelector should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_all_low_root_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (root_node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.querySelectorAll",
        "params": { "nodeId": root_node_id, "selector": ".fresh-child" }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.querySelectorAll should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn request_child_nodes_low_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": node_id, "depth": 2 }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.requestChildNodes should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_node_low_node_id_misses_without_frontend_binding() {
    let mut ctx = TestContext::new();
    let (node_id, mutation_completion) =
        append_live_node_without_refreshing_page_snapshot(&mut ctx).await;

    let raw = json!({
        "id": 3,
        "method": "DOM.removeNode",
        "params": { "nodeId": node_id }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("DOM.removeNode should verify frontend node binding in renderer");
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, *pending).await;
    assert_eq!(
        messages,
        vec![json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Could not find node with given id"
            }
        })]
    );

    let page = loaded_page_mut_for_test(&mut ctx);
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outer_html_and_scroll_low_node_refs_use_renderer_dispatch() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='target' data-state='ready' style='left:1px;top:2px;width:3px;height:4px'>go</section></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root id");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#target" }
    }))
    .await;
    let target_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .expect("target id");
    ctx.process_async(json!({
        "id": 31,
        "method": "DOM.describeNode",
        "params": { "nodeId": target_node_id }
    }))
    .await;
    let target_backend_node_id =
        take_response_by_id(&mut ctx, 31)["result"]["node"]["backendNodeId"]
            .as_u64()
            .expect("target backend id");

    let outer_raw = json!({
        "id": 4,
        "method": "DOM.getOuterHTML",
        "params": { "nodeId": target_node_id }
    })
    .to_string();
    let outer_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&outer_raw)
        .expect("low nodeId DOM.getOuterHTML should start a renderer command");
    assert_eq!(outer_pending.kind_name(), "DOM");
    let outer = complete_pending_command_task_for_test(&mut ctx, outer_pending).await;
    assert_eq!(outer.len(), 1);
    assert!(
        outer[0]["result"]["outerHTML"]
            .as_str()
            .is_some_and(|html| html.contains("data-state=\"ready\""))
    );

    let scroll_raw = json!({
        "id": 5,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "backendNodeId": target_backend_node_id }
    })
    .to_string();
    let scroll_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&scroll_raw)
        .expect("backendNodeId DOM.scrollIntoViewIfNeeded should start a renderer command");
    assert_eq!(scroll_pending.kind_name(), "DOM");
    let scroll = complete_pending_command_task_for_test(&mut ctx, scroll_pending).await;
    assert_eq!(
        scroll,
        vec![json!({
            "id": 5,
            "result": {}
        })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_returns_children_and_attributes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body id='main'><p class='greet'>hello</p></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let doc = ctx.take_one();
    let root_id = doc["result"]["root"]["nodeId"].as_u64().unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": {
            "nodeId": root_id,
            "selector": "body"
        }
    }))
    .await;
    let body_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(body_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": {
            "nodeId": body_id,
            "depth": 1
        }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 4);
    assert_eq!(described["id"], json!(4));
    assert_eq!(described["result"]["node"]["nodeName"], json!("BODY"));
    assert_eq!(
        described["result"]["node"]["attributes"],
        json!(["id", "main"])
    );
    assert_eq!(described["result"]["node"]["childNodeCount"], json!(1));
    assert_eq!(
        described["result"]["node"]["children"][0]["nodeName"],
        json!("P")
    );
    assert_eq!(
        described["result"]["node"]["children"][0]["attributes"],
        json!(["class", "greet"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_low_node_id_reads_renderer_live_snapshot() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><main id='root' data-state='ready'><span id='child'>ok</span></main></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 2)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document node id");

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#root" }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .expect("queried node id");

    let raw = json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": node_id, "depth": 1 }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("low nodeId DOM.describeNode should start a renderer snapshot command");
    assert_eq!(pending.kind_name(), "DOM");
    let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;
    let described = messages
        .iter()
        .find(|message| message["id"] == json!(4))
        .unwrap_or_else(|| panic!("pending DOM.describeNode should respond: {messages:?}"));

    assert_eq!(described["result"]["node"]["nodeName"], json!("MAIN"));
    let attrs = interleaved_attributes_to_map(&described["result"]["node"]["attributes"]);
    assert_eq!(attrs.get("id").map(String::as_str), Some("root"));
    assert_eq!(attrs.get("data-state").map(String::as_str), Some("ready"));
    let backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("renderer snapshot should assign backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "low nodeId describeNode should use renderer backend id namespace: {described}"
    );
    let child = &described["result"]["node"]["children"][0];
    assert_eq!(child["nodeName"], json!("SPAN"));
    let child_backend_node_id = child["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("renderer child snapshot should assign backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(child_backend_node_id),
        "low nodeId describeNode should assign renderer backend ids to descendants: {described}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_supports_renderer_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><main id='root' data-state='ready'><span id='child'>ok</span></main></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let live_node = renderer_backend_node_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#root')",
        0,
    )
    .await;
    let backend_node_id = live_node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live node should return backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "live describeNode should use renderer backend id namespace: {live_node}"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.describeNode",
        "params": { "backendNodeId": backend_node_id, "depth": 1 }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 13);
    assert_eq!(described["result"]["node"]["nodeName"], json!("MAIN"));
    let attrs = interleaved_attributes_to_map(&described["result"]["node"]["attributes"]);
    assert_eq!(attrs.get("id").map(String::as_str), Some("root"));
    assert_eq!(attrs.get("data-state").map(String::as_str), Some("ready"));
    let child = &described["result"]["node"]["children"][0];
    assert_eq!(child["nodeName"], json!("SPAN"));
    let child_backend_node_id = child["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live child should return backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(child_backend_node_id),
        "backend-id describeNode should assign renderer backend ids to descendants: {described}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_renderer_backend_id_is_scoped_to_document_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><main id='old'>old</main></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let live_node = renderer_backend_node_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#old')",
        0,
    )
    .await;
    let backend_node_id = live_node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("old live node should return backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "old live node should use renderer backend id namespace: {live_node}"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.open(); document.write(\"<!doctype html><html><body><main id='fresh'>fresh</main></body></html>\"); document.close(); document.querySelector('#fresh')"
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
        "method": "DOM.describeNode",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    ctx.expect_error(14, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn request_node_resolves_runtime_object_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = execution_context_id;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.body" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!object_id.is_empty());

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.requestNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 12)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(node_id > 0);

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getOuterHTML",
        "params": { "nodeId": node_id }
    }))
    .await;
    let outer_html = ctx.take_one()["result"]["outerHTML"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert_eq!(outer_html, "<body><div id=\"box\">ok</div></body>");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outer_html_include_shadow_dom_is_command_local_across_all_references() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-OUTER-HTML-SHADOW");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        concat!(
            "<!doctype html><html><body>",
            "<x-host id='host'>light</x-host><input id='control'>",
            "<script>",
            "const host = document.getElementById('host');",
            "const root = host.attachShadow({",
            "mode: 'closed', delegatesFocus: true, serializable: true, ",
            "slotAssignment: 'manual', clonable: true, referenceTarget: 'target&'",
            "});",
            "root.innerHTML = '<span data-x=\"&amp;\">shadow &lt;</span>'",
            "  + '<x-inner>inner-light</x-inner>';",
            "root.querySelector('x-inner').attachShadow({mode: 'open'}).innerHTML = ",
            "  '<b>nested</b>';",
            "</script></body></html>"
        ),
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.enable"}))
        .await;
    ctx.expect_result(2, json!({}), None);
    ctx.process_async(json!({"id": 3, "method": "DOM.getDocument"}))
        .await;
    let document = take_response_by_id(&mut ctx, 3);
    let root_node_id = document["result"]["root"]["nodeId"]
        .as_u64()
        .expect("document frontend node id");

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_node_id, "selector": "#host" }
    }))
    .await;
    let host_node_id = take_query_selector_node_id(&mut ctx, 4);
    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_node_id, "selector": "#control" }
    }))
    .await;
    let control_node_id = take_query_selector_node_id(&mut ctx, 5);

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.describeNode",
        "params": { "nodeId": host_node_id }
    }))
    .await;
    let host_backend_node_id = take_response_by_id(&mut ctx, 6)["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("host backend node id");
    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.resolveNode",
        "params": { "nodeId": host_node_id }
    }))
    .await;
    let host_object_id = take_response_by_id(&mut ctx, 7)["result"]["object"]["objectId"]
        .as_str()
        .expect("host object id")
        .to_owned();

    let ordinary = "<x-host id=\"host\">light</x-host>";
    let including_shadow = concat!(
        "<x-host id=\"host\"><template shadowrootmode=\"closed\" ",
        "shadowrootdelegatesfocus=\"\" shadowrootserializable=\"\" ",
        "shadowrootclonable=\"\"><span data-x=\"&amp;\">shadow &lt;</span>",
        "<x-inner><template shadowrootmode=\"open\"><b>nested</b></template>",
        "inner-light</x-inner></template>light</x-host>"
    );

    ctx.sent.clear();
    for (id, params, expected) in [
        (10, json!({ "nodeId": host_node_id }), ordinary),
        (
            11,
            json!({ "nodeId": host_node_id, "includeShadowDOM": false }),
            ordinary,
        ),
        (
            12,
            json!({ "nodeId": host_node_id, "includeShadowDOM": true }),
            including_shadow,
        ),
        (
            13,
            json!({
                "backendNodeId": host_backend_node_id,
                "includeShadowDOM": true
            }),
            including_shadow,
        ),
        (
            14,
            json!({ "objectId": host_object_id, "includeShadowDOM": true }),
            including_shadow,
        ),
        (
            15,
            json!({ "objectId": host_object_id, "includeShadowDOM": false }),
            ordinary,
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "DOM.getOuterHTML",
            "params": params
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(
            response["result"]["outerHTML"],
            json!(expected),
            "{response}"
        );
    }
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("method").is_none()),
        "read-only outerHTML commands must not emit DOM events: {:?}",
        ctx.sent
    );

    ctx.process_async(json!({
        "id": 16,
        "method": "DOM.getOuterHTML",
        "params": { "nodeId": control_node_id, "includeShadowDOM": true }
    }))
    .await;
    let control = take_response_by_id(&mut ctx, 16);
    assert_eq!(
        control["result"]["outerHTML"],
        json!("<input id=\"control\">")
    );

    let (document_result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::Cdp,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            reference: None,
            include_shadow_dom: true,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetOuterHtml(document_result) =
        document_result.expect("document outerHTML command should succeed")
    else {
        panic!("expected document outerHTML result");
    };
    assert!(document_result.outer_html.contains(including_shadow));
    assert_eq!(
        document_result
            .outer_html
            .matches("<template shadowrootmode=")
            .count(),
        2,
        "document output should include exactly the closed outer and nested open author roots"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_request_node_resolves_runtime_object_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.body" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!object_id.is_empty());

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.requestNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 12)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(node_id > 0);

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getOuterHTML",
        "params": { "nodeId": node_id }
    }))
    .await;
    let outer_html = ctx.take_one()["result"]["outerHTML"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert_eq!(outer_html, "<body><div id=\"box\">ok</div></body>");
}

#[tokio::test(flavor = "multi_thread")]
async fn object_snapshot_producers_register_renderer_frontend_bindings() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='request-target' data-source='request'></section><article id='describe-target' data-source='describe'></article></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#request-target')" }
    }))
    .await;
    let request_object_id = take_response_by_id(&mut ctx, 11)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("request target Runtime.evaluate should return objectId"))
        .to_owned();

    ctx.process_async(json!({
        "id": 12,
        "method": "DOM.requestNode",
        "params": { "objectId": request_object_id }
    }))
    .await;
    let request_frontend_node_id = take_response_by_id(&mut ctx, 12)["result"]["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("requestNode should return frontend node id");

    ctx.conn
        .clear_runtime_remote_object_tracking_for_session_owner(None);
    let request_backend_node_id =
        match renderer_frontend_binding_for_test(&mut ctx, request_frontend_node_id).await {
            moli_core::page::RendererDomFrontendNodeBindingResolution::BackendNodeId(
                backend_node_id,
            ) => backend_node_id,
            resolution => {
                panic!("requestNode producer should register renderer binding, got {resolution:?}")
            }
        };
    assert!(
        moli_core::page::is_renderer_backend_node_id(request_backend_node_id),
        "requestNode producer should bind frontend node to renderer-owned backend id"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#describe-target')" }
    }))
    .await;
    let describe_object_id = take_response_by_id(&mut ctx, 13)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("describe target Runtime.evaluate should return objectId"))
        .to_owned();

    ctx.process_async(json!({
        "id": 14,
        "method": "DOM.describeNode",
        "params": { "objectId": describe_object_id, "depth": 0 }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 14);
    let describe_frontend_node_id = described["result"]["node"]["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("describeNode should return frontend node id");
    let describe_backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("describeNode should return backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(describe_backend_node_id),
        "describeNode producer should return renderer-owned backend id"
    );

    ctx.conn
        .clear_runtime_remote_object_tracking_for_session_owner(None);
    let bound_describe_backend_node_id =
        match renderer_frontend_binding_for_test(&mut ctx, describe_frontend_node_id).await {
            moli_core::page::RendererDomFrontendNodeBindingResolution::BackendNodeId(
                backend_node_id,
            ) => backend_node_id,
            resolution => {
                panic!("describeNode producer should register renderer binding, got {resolution:?}")
            }
        };
    assert_eq!(
        bound_describe_backend_node_id, describe_backend_node_id,
        "renderer DOM agent binding should preserve the exact backend id returned by describeNode"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn request_node_object_id_reads_live_document_after_document_open_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='old' data-owner='initial'></body></html>",
    )
    .await;
    ctx.process_async(json!({ "id": 9, "method": "DOM.enable" }))
        .await;
    ctx.expect_result(9, json!({}), None);

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
                document.write("<!doctype html><html><body><input id='fresh-request' data-owner='replacement' value='new'><span id='tail'>tail</span></body></html>");
                document.close();
                return document.querySelector('#fresh-request');
            })()"#
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 12)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("replacement Runtime.evaluate should return objectId"))
        .to_owned();
    crate::testing::wait_until_message(
        &mut ctx,
        None,
        "document.open DOMContentLoaded binding refresh",
        |message| message["method"] == json!("DOM.documentUpdated"),
    )
    .await;

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.requestNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let node_id = take_response_by_id(&mut ctx, 13)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(node_id > 0);

    ctx.process_async(json!({
        "id": 14,
        "method": "DOM.describeNode",
        "params": { "nodeId": node_id, "depth": 0 }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 14);
    assert_eq!(described["result"]["node"]["nodeName"], json!("INPUT"));
    let attrs = interleaved_attributes_to_map(&described["result"]["node"]["attributes"]);
    assert_eq!(attrs.get("id").map(String::as_str), Some("fresh-request"));
    assert_eq!(
        attrs.get("data-owner").map(String::as_str),
        Some("replacement")
    );
    assert_eq!(attrs.get("value").map(String::as_str), Some("new"));
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_object_id_reads_live_document_after_document_open_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='old' data-owner='initial'></body></html>",
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
                document.write("<!doctype html><html><body><input id='fresh' data-owner='replacement' value='new'><span id='tail'>tail</span></body></html>");
                document.close();
                return document.querySelector('#fresh');
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
        "method": "DOM.describeNode",
        "params": { "objectId": object_id, "depth": 0 }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 13);
    assert_eq!(described["result"]["node"]["nodeName"], json!("INPUT"));
    let attrs = interleaved_attributes_to_map(&described["result"]["node"]["attributes"]);
    assert_eq!(attrs.get("id").map(String::as_str), Some("fresh"));
    assert_eq!(
        attrs.get("data-owner").map(String::as_str),
        Some("replacement")
    );
    assert_eq!(attrs.get("value").map(String::as_str), Some("new"));
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_object_id_assigns_renderer_backend_ids_to_live_subtree() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='root'><span id='child'>ok</span></div></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#root')" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 2)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("root element should return an objectId");

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id, "depth": 1 }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 3);
    let root_backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live root should return u32 backendNodeId");
    let child_backend_node_id = described["result"]["node"]["children"][0]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live child should return u32 backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(root_backend_node_id),
        "live root backend id should use renderer namespace: {described}"
    );
    assert!(
        moli_core::page::is_renderer_backend_node_id(child_backend_node_id),
        "live child backend id should use renderer namespace: {described}"
    );
    assert_ne!(root_backend_node_id, child_backend_node_id);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": child_backend_node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 4);
    assert_eq!(resolved["result"]["object"]["subtype"], json!("node"));
    let child_object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("child backend id should resolve to an objectId");

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": child_object_id,
            "functionDeclaration": "function() { return [this.localName, this.id, this.textContent].join('|'); }",
            "returnByValue": true
        }
    }))
    .await;
    let checked = take_response_by_id(&mut ctx, 5);
    assert_eq!(checked["result"]["result"]["value"], json!("span|child|ok"));
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_object_id_assigns_renderer_backend_ids_to_live_shadow_tree() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<span id=\"inside\">shadow</span>';</script></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#host')" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 2)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("host element should return an objectId");

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.describeNode",
        "params": {
            "objectId": object_id,
            "depth": 2,
            "pierce": true
        }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 3);
    let host_backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live host should return u32 backendNodeId");
    let shadow_root_backend_node_id =
        described["result"]["node"]["shadowRoots"][0]["backendNodeId"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .expect("live shadow root should return u32 backendNodeId");
    let shadow_child_backend_node_id = described["result"]["node"]["shadowRoots"][0]["children"][0]
        ["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live shadow child should return u32 backendNodeId");
    for backend_node_id in [
        host_backend_node_id,
        shadow_root_backend_node_id,
        shadow_child_backend_node_id,
    ] {
        assert!(
            moli_core::page::is_renderer_backend_node_id(backend_node_id),
            "live shadow tree backend id should use renderer namespace: {described}"
        );
    }
    assert_ne!(host_backend_node_id, shadow_root_backend_node_id);
    assert_ne!(host_backend_node_id, shadow_child_backend_node_id);
    assert_ne!(shadow_root_backend_node_id, shadow_child_backend_node_id);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": shadow_child_backend_node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 4);
    assert_eq!(resolved["result"]["object"]["subtype"], json!("node"));
    let shadow_child_object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("shadow child backend id should resolve to an objectId");

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": shadow_child_object_id,
            "functionDeclaration": "function() { return [this.localName, this.id, this.textContent].join('|'); }",
            "returnByValue": true
        }
    }))
    .await;
    let checked = take_response_by_id(&mut ctx, 5);
    assert_eq!(
        checked["result"]["result"]["value"],
        json!("span|inside|shadow")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn query_selector_accepts_renderer_backend_shadow_root_reference() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<span id=\"inside\">shadow</span>';</script></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#host')" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 2)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("host element should return an objectId");
    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.describeNode",
        "params": {
            "objectId": object_id,
            "depth": 1,
            "pierce": true
        }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 3);
    let shadow_root_backend_node_id =
        described["result"]["node"]["shadowRoots"][0]["backendNodeId"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .expect("live shadow root should return u32 backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(shadow_root_backend_node_id),
        "shadow root backend id should be renderer-owned: {described}"
    );

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::Cdp,
        session_id: None,
        target_id: Some(DevToolsTargetId::from("TID-1")),
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-A")),
    };
    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: context.clone(),
                root: Some(DevToolsDomNodeReference::BackendNodeId(
                    shadow_root_backend_node_id,
                )),
                selector: "#inside".to_owned(),
                multiple: false,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("shadow root backend querySelector should run");
    let DevToolsCommandResult::QuerySelector(result) = result else {
        panic!("expected query selector result");
    };
    assert_eq!(result.node_ids.len(), 1);
    let found_node_id = result.node_ids[0];
    assert!(found_node_id > 0);
    let described = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context,
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(found_node_id)),
            depth: 0,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child shadow query result nodeId should be describable");
    let DevToolsCommandResult::DescribeNode(described) = described else {
        panic!("expected describe node result");
    };
    assert_eq!(described.node["attributes"][0], "id");
    assert_eq!(described.node["attributes"][1], "inside");
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_query_selector_accepts_renderer_backend_shadow_root_reference() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-CHILD-SHADOW-QUERY");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe srcdoc=\"<body><div id='host'></div></body>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.enable"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3);
    let child_context_id = child_default_context_id_from_events(&ctx, &child_frame_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": child_context_id,
            "expression": "(() => { const host = document.querySelector('#host'); const root = host.attachShadow({mode:'closed'}); root.innerHTML = '<span id=\"inside\">shadow</span>'; return host; })()"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("child host element should return an objectId");
    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.describeNode",
        "params": {
            "objectId": object_id,
            "depth": 1,
            "pierce": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5);

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::Cdp,
        session_id: None,
        target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-CHILD-SHADOW-QUERY")),
    };
    let host_query = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: context.clone(),
                root: None,
                selector: "#host".to_owned(),
                multiple: false,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("child host querySelector should run");
    let DevToolsCommandResult::QuerySelector(host_query) = host_query else {
        panic!("expected query selector result");
    };
    let host_frontend_node_id = host_query
        .node_ids
        .first()
        .copied()
        .expect("child host query should return frontend node id");

    let frontend_described = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(
                host_frontend_node_id,
            )),
            depth: 1,
            pierce: true,
        }))
        .await
        .into_parts()
        .0
        .expect("child host frontend describe should run");
    let DevToolsCommandResult::DescribeNode(frontend_described) = frontend_described else {
        panic!("expected describe node result");
    };
    let shadow_root_backend_node_id = frontend_described.node["shadowRoots"][0]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("child frontend host describe should return shadow root backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(shadow_root_backend_node_id),
        "child shadow root backend id should be renderer-owned: {frontend_described:?}"
    );
    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context,
                root: Some(DevToolsDomNodeReference::BackendNodeId(
                    shadow_root_backend_node_id,
                )),
                selector: "#inside".to_owned(),
                multiple: false,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("child shadow root backend querySelector should run");
    let DevToolsCommandResult::QuerySelector(result) = result else {
        panic!("expected query selector result");
    };
    assert_eq!(result.node_ids.len(), 1);
    assert!(result.node_ids[0] > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_describe_backend_host_returns_queryable_shadow_root_backend_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-CHILD-SHADOW-DESCRIBE");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe srcdoc=\"<body><div id='host'></div></body>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.enable"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3);
    let child_context_id = child_default_context_id_from_events(&ctx, &child_frame_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": child_context_id,
            "expression": "(() => { const host = document.querySelector('#host'); const root = host.attachShadow({mode:'closed'}); root.innerHTML = '<span id=\"inside\">shadow</span>'; return host; })()"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("child host element should return an objectId");
    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let host_backend_node_id = take_response_by_id(&mut ctx, 5)["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("child host should return backendNodeId");

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::Cdp,
        session_id: None,
        target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-CHILD-SHADOW-DESCRIBE")),
    };
    let described = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::BackendNodeId(
                host_backend_node_id,
            )),
            depth: 1,
            pierce: true,
        }))
        .await
        .into_parts()
        .0
        .expect("child host backend describe should run");
    let DevToolsCommandResult::DescribeNode(described) = described else {
        panic!("expected describe node result");
    };
    let shadow_root_backend_node_id = described.node["shadowRoots"][0]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("child backend host describe should return shadow root backendNodeId");

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context,
                root: Some(DevToolsDomNodeReference::BackendNodeId(
                    shadow_root_backend_node_id,
                )),
                selector: "#inside".to_owned(),
                multiple: false,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("child shadow root backend from backend describe should query");
    let DevToolsCommandResult::QuerySelector(result) = result else {
        panic!("expected query selector result");
    };
    assert_eq!(result.node_ids.len(), 1);
    assert!(result.node_ids[0] > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outer_html_object_id_reads_live_document_after_document_open_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='old' data-owner='initial'>old</section></body></html>",
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
                document.write("<!doctype html><html><body><section id='fresh-outer' data-owner='replacement'><p>new &amp; live</p></section><span id='tail'>tail</span></body></html>");
                document.close();
                return document.querySelector('#fresh-outer');
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
        "method": "DOM.getOuterHTML",
        "params": { "objectId": object_id }
    }))
    .await;
    let outer_html = take_response_by_id(&mut ctx, 13)["result"]["outerHTML"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert_eq!(
        outer_html,
        "<section id=\"fresh-outer\" data-owner=\"replacement\"><p>new &amp; live</p></section>"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outer_html_supports_renderer_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='outer' data-state='ready'><p>hello &amp; live</p></section></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let live_node = renderer_backend_node_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#outer')",
        0,
    )
    .await;
    let backend_node_id = live_node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live node should return backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "live node should use renderer backend id namespace: {live_node}"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "DOM.getOuterHTML",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    let outer_html = take_response_by_id(&mut ctx, 13)["result"]["outerHTML"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert_eq!(
        outer_html,
        "<section id=\"outer\" data-state=\"ready\"><p>hello &amp; live</p></section>"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outer_html_renderer_backend_id_is_scoped_to_document_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='old'>old</section></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    let live_node = renderer_backend_node_for_live_expression(
        &mut ctx,
        11,
        12,
        "document.querySelector('#old')",
        0,
    )
    .await;
    let backend_node_id = live_node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("old live node should return backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "old live node should use renderer backend id namespace: {live_node}"
    );

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.open(); document.write(\"<!doctype html><html><body><section id='fresh'>fresh</section></body></html>\"); document.close(); document.querySelector('#fresh')"
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
        "method": "DOM.getOuterHTML",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    ctx.expect_error(14, -32000, "Could not find node with given id");
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_returns_runtime_object_for_frontend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

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
    assert!(node_id > 0);

    let resolve_raw = json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": node_id,
            "executionContextId": execution_context_id
        }
    })
    .to_string();
    let resolve_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&resolve_raw)
        .expect("DOM.resolveNode with executionContextId should start as a pending command");
    let messages = complete_pending_command_task_for_test(&mut ctx, resolve_pending).await;
    let msg = messages
        .iter()
        .find(|message| message["id"] == json!(4))
        .expect("pending DOM.resolveNode should produce a response");
    assert_eq!(msg["id"], json!(4));
    assert_eq!(msg["result"]["object"]["type"], json!("object"));
    assert_eq!(msg["result"]["object"]["subtype"], json!("node"));
    assert!(msg["result"]["object"]["objectId"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_caches_node_payload_from_renderer_snapshot_command() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
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
    assert!(node_id > 0);

    let resolve_raw = json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": { "nodeId": node_id }
    })
    .to_string();
    let resolve_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&resolve_raw)
        .expect("DOM.resolveNode should start a renderer runtime-object command");
    let completed = resolve_pending.wait().await;
    let cache_pending = match ctx.conn.complete_pending_command_dispatch(completed).await {
        CdpCommandTaskStep::Pending(pending) => *pending,
        CdpCommandTaskStep::Complete(outcome) => {
            panic!(
                "DOM.resolveNode should wait for renderer cache snapshot command: {:?}",
                outcome.into_parts().0
            )
        }
    };
    let messages = complete_pending_command_task_for_test(&mut ctx, cache_pending).await;
    let resolved = messages
        .iter()
        .find(|message| message["id"] == json!(4))
        .unwrap_or_else(|| panic!("pending DOM.resolveNode should respond: {messages:?}"));
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .expect("DOM.resolveNode should return objectId")
        .to_owned();

    navigate_to_data_html_async(&mut ctx, 5, "<!doctype html><html><body>next</body></html>").await;

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 6);
    assert_eq!(described["result"]["node"]["nodeName"], json!("DIV"));
    assert_eq!(
        described["result"]["node"]["attributes"],
        json!(["id", "box"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_uses_cached_resolved_node_after_navigation_until_release() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

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
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 4)["result"]["object"]["objectId"]
        .as_str()
        .expect("resolveNode should return objectId")
        .to_owned();

    navigate_to_data_html_async(&mut ctx, 5, "<!doctype html><html><body>next</body></html>").await;

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id.clone() }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 6);
    assert_eq!(described["result"]["node"]["nodeName"], json!("DIV"));
    assert_eq!(
        described["result"]["node"]["attributes"],
        json!(["id", "box"])
    );
    let backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .expect("cached describeNode should preserve backendNodeId");

    ctx.process_async(json!({
        "id": 6_1,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": backend_node_id,
            "executionContextId": 1000000
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 6_1);
    assert!(
        resolved["error"]["message"] == json!("InvalidParam")
            || resolved["error"]["message"] == json!("ContextNotFound")
            || resolved["error"]["message"] == json!("Could not find node with given id"),
        "backendNodeId caches must not resurrect stale remote objects across navigation: {resolved}"
    );

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.releaseObject",
        "params": { "objectId": object_id.clone() }
    }))
    .await;
    let release = take_response_by_id(&mut ctx, 7);
    assert!(
        release.get("result").is_some() || release.get("error").is_some(),
        "releaseObject should produce a terminal CDP response"
    );

    ctx.process_async(json!({
        "id": 8,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let described_after_release = take_response_by_id(&mut ctx, 8);
    assert_eq!(
        described_after_release["result"]["node"]["nodeName"],
        json!("DIV")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_renderer_backend_node_id_is_scoped_to_document_replacement() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input id='old'></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#old')" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 2)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("old input should return an objectId");

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 3);
    let backend_node_id = described["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("live object describeNode should return u32 backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "objectId describeNode should use renderer backend id namespace: {described}"
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": backend_node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let resolved_before = take_response_by_id(&mut ctx, 4);
    assert_eq!(
        resolved_before["result"]["object"]["subtype"],
        json!("node")
    );
    assert!(
        resolved_before["result"]["object"]["objectId"]
            .as_str()
            .is_some(),
        "renderer backend id should resolve before replacement: {resolved_before}"
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.open(); document.write(\"<!doctype html><html><body><input id='fresh'></body></html>\"); document.close(); document.querySelector('#fresh')"
        }
    }))
    .await;
    let fresh = take_response_by_id(&mut ctx, 5);
    assert!(
        fresh["result"]["result"]["objectId"].as_str().is_some(),
        "replacement input should produce a fresh object: {fresh}"
    );

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": backend_node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let resolved_after = take_response_by_id(&mut ctx, 6);
    assert_eq!(
        resolved_after["error"]["message"],
        json!("Could not find node with given id"),
        "stale renderer backend id must not resolve against the replacement document: {resolved_after}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_resolve_node_returns_runtime_object_for_frontend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

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
    assert!(node_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let msg = take_response_by_id(&mut ctx, 4);
    assert_eq!(msg["id"], json!(4));
    assert_eq!(msg["result"]["object"]["type"], json!("object"));
    assert_eq!(msg["result"]["object"]["subtype"], json!("node"));
    assert!(msg["result"]["object"]["objectId"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_with_execution_context_returns_object_bound_to_command_session() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.attach_active_session("SID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "sessionId": "SID-1",
        "method": "Runtime.enable"
    }))
    .await;
    let runtime_enabled = take_response_by_id(&mut ctx, 2);
    assert_eq!(runtime_enabled["result"], json!({}));
    let execution_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-1")
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .unwrap_or_else(|| panic!("Runtime.enable should emit default context: {:?}", ctx.sent));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3,
        "sessionId": "SID-1",
        "method": "DOM.getDocument"
    }))
    .await;
    let root_id = take_response_by_id(&mut ctx, 3)["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(root_id > 0);

    ctx.process_async(json!({
        "id": 4,
        "sessionId": "SID-1",
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#box" }
    }))
    .await;
    let node_id = take_query_selector_node_id(&mut ctx, 4);
    assert!(node_id > 0);

    ctx.process_async(json!({
        "id": 5,
        "sessionId": "SID-1",
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": node_id,
            "executionContextId": execution_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 5);
    assert_eq!(resolved["sessionId"], json!("SID-1"));
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("DOM.resolveNode should return objectId: {resolved}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 6,
        "sessionId": "SID-1",
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id,
            "returnByValue": true,
            "functionDeclaration": "function() { return [this.nodeType, this.constructor && this.constructor.name, typeof this.remove].join('|'); }"
        }
    }))
    .await;
    let called = take_response_by_id(&mut ctx, 6);
    assert_eq!(called["sessionId"], json!("SID-1"));
    assert_eq!(
        called["result"]["result"]["value"],
        json!("1|HTMLDivElement|function")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_resolve_node_with_execution_context_caches_top_frame_id_for_html_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = ctx.take_one()["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "html" }
    }))
    .await;
    let html_node_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(html_node_id > 0);

    let resolve_raw = json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": html_node_id,
            "executionContextId": execution_context_id
        }
    })
    .to_string();
    let resolve_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&resolve_raw)
        .expect("DOM.resolveNode with executionContextId should start as a pending command");
    let resolve_messages = complete_pending_command_task_for_test(&mut ctx, resolve_pending).await;
    let resolved = resolve_messages
        .iter()
        .find(|message| message["id"] == json!(4))
        .expect("pending DOM.resolveNode should produce a response");
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .expect("DOM.resolveNode should return objectId")
        .to_owned();

    navigate_to_data_html_async(&mut ctx, 5, "<!doctype html><html><body>next</body></html>").await;

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 6);
    assert_eq!(described["result"]["node"]["nodeName"], json!("HTML"));
    assert_eq!(described["result"]["node"]["frameId"], json!("TID-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_supports_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

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
        "method": "DOM.resolveNode",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    let msg = take_response_by_id(&mut ctx, 5);
    assert_eq!(msg["id"], json!(5));
    assert_eq!(msg["result"]["object"]["type"], json!("object"));
    assert_eq!(msg["result"]["object"]["subtype"], json!("node"));
    assert!(msg["result"]["object"]["objectId"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_rejects_unknown_execution_context() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 10).await;
    let _ = ctx.take_all();

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
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": node_id,
            "executionContextId": 999999
        }
    }))
    .await;
    ctx.expect_error(4, -32000, "ContextNotFound");
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_requires_node_reference() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='box'>ok</div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.resolveNode",
        "params": {}
    }))
    .await;
    ctx.expect_error(2, -32602, "InvalidParam");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outer_html_supports_object_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section><p>hello &amp; bye</p></section></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 21,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('section')" }
    }))
    .await;
    let object_id = ctx.take_one()["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    ctx.process_async(json!({
        "id": 22,
        "method": "DOM.getOuterHTML",
        "params": { "objectId": object_id }
    }))
    .await;
    let outer_html = ctx.take_one()["result"]["outerHTML"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert_eq!(outer_html, "<section><p>hello &amp; bye</p></section>");
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_accepts_isolated_execution_context_for_closed_shadow_root_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>document.getElementById('host').attachShadow({mode:'closed'});</script></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": "TID-1", "worldName": "patchright-utility" }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    assert!(isolated_context_id > 0);
    let _ = ctx.take_all();

    ctx.process_async(
        json!({"id": 4, "method": "DOM.getDocument", "params": { "pierce": true, "depth": -1 }}),
    )
    .await;
    let described = take_response_by_id(&mut ctx, 4);
    let shadow_root_backend_id =
        patchright_collect_closed_shadow_root_backend_ids(&described["result"]["root"])
            .into_iter()
            .next()
            .unwrap_or_default();
    assert!(shadow_root_backend_id > 0);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": shadow_root_backend_id,
            "executionContextId": isolated_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 5);
    assert_eq!(resolved["result"]["object"]["type"], json!("object"));
    assert!(resolved["result"]["object"]["objectId"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_accepts_context_id_alias_for_isolated_execution_context() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>document.getElementById('host').attachShadow({mode:'closed'});</script></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": "TID-1", "worldName": "patchright-utility" }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    assert!(isolated_context_id > 0);
    let _ = ctx.take_all();

    ctx.process_async(
        json!({"id": 4, "method": "DOM.getDocument", "params": { "pierce": true, "depth": -1 }}),
    )
    .await;
    let described = take_response_by_id(&mut ctx, 4);
    let shadow_root_backend_id =
        patchright_collect_closed_shadow_root_backend_ids(&described["result"]["root"])
            .into_iter()
            .next()
            .unwrap_or_default();
    assert!(shadow_root_backend_id > 0);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": shadow_root_backend_id,
            "contextId": isolated_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 5);
    assert_eq!(resolved["result"]["object"]["type"], json!("object"));
    assert!(resolved["result"]["object"]["objectId"].as_str().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_context_id_alias_patchright_xpath_engine_returns_live_element_handles() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<section><div id=\"a\"></div><span><div id=\"b\"></div></span></section>';</script></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.createIsolatedWorld",
        "params": { "frameId": "TID-1", "worldName": "patchright-utility" }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    assert!(isolated_context_id > 0);
    let _ = ctx.take_all();

    ctx.process_async(
        json!({"id": 4, "method": "DOM.getDocument", "params": { "pierce": true, "depth": -1 }}),
    )
    .await;
    let described_document = take_response_by_id(&mut ctx, 4)["result"]["root"].clone();
    let shadow_root_backend_id =
        patchright_collect_closed_shadow_root_backend_ids(&described_document)
            .into_iter()
            .next()
            .unwrap_or_default();
    assert!(shadow_root_backend_id > 0);

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": shadow_root_backend_id,
            "contextId": isolated_context_id
        }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 5);
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!object_id.is_empty());

    ctx.process_async(json!({
            "id": 6,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "arguments": [{ "value": "//div" }],
                "functionDeclaration": r#"function(selector) {
                    if (this.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return [];
                    const result = [];
                    const parser = new DOMParser();
                    function getAllChildElements(node) {
                        const elements = [];
                        const traverse = currentNode => {
                            if (currentNode.nodeType === Node.ELEMENT_NODE) elements.push(currentNode);
                            currentNode.childNodes?.forEach(traverse);
                        };
                        if (node.nodeType === Node.DOCUMENT_FRAGMENT_NODE || node.nodeType === Node.ELEMENT_NODE) traverse(node);
                        return elements;
                    }
                    const csrHTMLContent = this.innerHTML;
                    const csrChildElements = getAllChildElements(this);
                    const htmlDoc = parser.parseFromString(csrHTMLContent, 'text/html');
                    const rootDiv = htmlDoc.body;
                    const rootDivChildElements = getAllChildElements(rootDiv);
                    const it = htmlDoc.evaluate(selector, htmlDoc, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE);
                    for (let node = it.iterateNext(); node; node = it.iterateNext()) {
                        const nodeIndex = rootDivChildElements.indexOf(node) - 1;
                        if (nodeIndex >= 0) {
                            const originalNode = csrChildElements[nodeIndex];
                            if (originalNode.nodeType === Node.ELEMENT_NODE) result.push(originalNode);
                        }
                    }
                    return result;
                }"#
            }
        })).await;
    let selection = take_response_by_id(&mut ctx, 6);
    assert_eq!(selection["result"]["result"]["subtype"], json!("array"));
    let array_object_id = selection["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!array_object_id.is_empty());

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.getProperties",
        "params": { "objectId": array_object_id, "ownProperties": true }
    }))
    .await;
    let properties = take_response_by_id(&mut ctx, 7)["result"]["result"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let first_element_object_id = properties
        .iter()
        .find(|property| property["name"] == json!("0"))
        .and_then(|property| property["value"]["objectId"].as_str())
        .unwrap_or_default()
        .to_owned();
    let second_element_object_id = properties
        .iter()
        .find(|property| property["name"] == json!("1"))
        .and_then(|property| property["value"]["objectId"].as_str())
        .unwrap_or_default()
        .to_owned();
    assert!(!first_element_object_id.is_empty());
    assert!(!second_element_object_id.is_empty());

    ctx.process_async(json!({
        "id": 8,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": first_element_object_id,
            "returnByValue": true,
            "functionDeclaration": "function() { return this.id; }"
        }
    }))
    .await;
    let first_id = take_response_by_id(&mut ctx, 8)["result"]["result"]["value"].clone();
    ctx.process_async(json!({
        "id": 9,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": second_element_object_id,
            "returnByValue": true,
            "functionDeclaration": "function() { return this.id; }"
        }
    }))
    .await;
    let second_id = take_response_by_id(&mut ctx, 9)["result"]["result"]["value"].clone();
    assert_eq!(first_id, json!("a"));
    assert_eq!(second_id, json!("b"));
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_with_child_default_execution_context_uses_child_document_backend_ids() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-CHILD-RESOLVE");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><input value='main'><iframe srcdoc=\"<body><div id='child' style='left:3px;top:4px;width:9px;height:11px'>child-body</div></body>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;

    ctx.process_async(json!({
        "id": 3,
        "method": "Runtime.enable"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3);
    let child_context_id = child_default_context_id_from_events(&ctx, &child_frame_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3_1,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": child_context_id,
            "expression": "document.body"
        }
    }))
    .await;
    let direct_child_object_id = take_response_by_id(&mut ctx, 3_1)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("child Runtime.evaluate should return body object id"));
    ctx.process_async(json!({
        "id": 3_2,
        "method": "DOM.describeNode",
        "params": { "objectId": direct_child_object_id.clone(), "depth": 1 }
    }))
    .await;
    let direct_child_describe = take_response_by_id(&mut ctx, 3_2);
    assert_eq!(
        direct_child_describe["result"]["node"]["nodeName"],
        json!("BODY")
    );
    assert_eq!(
        direct_child_describe["result"]["node"]["frameId"],
        json!(child_frame_id)
    );
    assert_eq!(
        direct_child_describe["result"]["node"]["children"][0]["nodeName"],
        json!("DIV")
    );
    let child_body_renderer_backend_id = direct_child_describe["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| {
            panic!("child describeNode should return backendNodeId: {direct_child_describe}")
        });
    assert!(
        moli_core::page::is_renderer_backend_node_id(child_body_renderer_backend_id),
        "child object describe should return renderer-owned backend id: {direct_child_describe}"
    );

    let high_backend_resolve_raw = json!({
        "id": 3_25,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": child_body_renderer_backend_id,
            "executionContextId": child_context_id
        }
    })
    .to_string();
    let high_backend_resolve_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&high_backend_resolve_raw)
        .expect(
            "child-frame DOM.resolveNode with renderer backend id should enter command task path",
        );
    let high_backend_resolve_messages =
        complete_pending_command_task_for_test(&mut ctx, high_backend_resolve_pending).await;
    let high_backend_resolved = high_backend_resolve_messages
        .iter()
        .find(|message| message["id"] == json!(3_25))
        .unwrap_or_else(|| {
            panic!(
                "pending child-frame high backend DOM.resolveNode should produce a response: {high_backend_resolve_messages:?}"
            )
        });
    let high_backend_object_id = high_backend_resolved["result"]["object"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            panic!(
                "child high-backend DOM.resolveNode should return objectId: {high_backend_resolved}"
            )
        });
    ctx.process_async(json!({
        "id": 3_26,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": high_backend_object_id,
            "functionDeclaration": "function() { return this.textContent.trim(); }"
        }
    }))
    .await;
    let high_backend_text = take_response_by_id(&mut ctx, 3_26);
    assert_eq!(
        high_backend_text["result"]["result"]["value"],
        json!("child-body")
    );

    ctx.process_async(json!({
        "id": 3_3,
        "method": "DOM.requestNode",
        "params": { "objectId": direct_child_object_id.clone() }
    }))
    .await;
    let direct_child_request = take_response_by_id(&mut ctx, 3_3);
    assert_eq!(
        direct_child_request["result"]["nodeId"],
        direct_child_describe["result"]["node"]["nodeId"]
    );

    ctx.process_async(json!({
        "id": 3_4,
        "method": "DOM.getOuterHTML",
        "params": { "objectId": direct_child_object_id }
    }))
    .await;
    let direct_child_outer_html = take_response_by_id(&mut ctx, 3_4);
    assert_eq!(
        direct_child_outer_html["result"]["outerHTML"],
        json!(
            "<body><div id=\"child\" style=\"left:3px;top:4px;width:9px;height:11px\">child-body</div></body>"
        )
    );

    let child_body_frontend_node_id = direct_child_describe["result"]["node"]["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| {
            panic!("child describeNode should return frontend nodeId: {direct_child_describe}")
        });
    assert!(
        child_body_frontend_node_id > 0,
        "child describeNode should return a bound frontend nodeId: {direct_child_describe}"
    );
    let child_body_backend_id = child_body_renderer_backend_id;

    let resolve_raw = json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": child_body_backend_id,
            "executionContextId": child_context_id
        }
    })
    .to_string();
    let resolve_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&resolve_raw)
        .expect(
            "DOM.resolveNode with child default executionContextId should enter command task path",
        );
    let resolve_messages = complete_pending_command_task_for_test(&mut ctx, resolve_pending).await;
    let resolved = resolve_messages
        .iter()
        .find(|message| message["id"] == json!(4))
        .unwrap_or_else(|| {
            panic!("pending child-frame DOM.resolveNode should produce a response: {resolve_messages:?}")
        });
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("DOM.resolveNode should return a remote object: {resolved}"));

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "function() { return this.textContent.trim(); }"
        }
    }))
    .await;
    let text = take_response_by_id(&mut ctx, 5);
    assert_eq!(text["result"]["result"]["value"], json!("child-body"));

    let frontend_resolve_raw = json!({
        "id": 5_1,
        "method": "DOM.resolveNode",
        "params": {
            "nodeId": child_body_frontend_node_id,
            "executionContextId": child_context_id
        }
    })
    .to_string();
    let frontend_resolve_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&frontend_resolve_raw)
        .expect(
            "DOM.resolveNode with child default executionContextId and frontend node id should enter command task path",
        );
    let frontend_resolve_messages =
        complete_pending_command_task_for_test(&mut ctx, frontend_resolve_pending).await;
    let frontend_resolved = frontend_resolve_messages
        .iter()
        .find(|message| message["id"] == json!(5_1))
        .unwrap_or_else(|| {
            panic!(
                "pending child-frame frontend DOM.resolveNode should produce a response: {frontend_resolve_messages:?}"
            )
        });
    let frontend_object_id = frontend_resolved["result"]["object"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            panic!(
                "child-frame frontend DOM.resolveNode should return objectId: {frontend_resolved}"
            )
        });
    ctx.process_async(json!({
        "id": 5_2,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": frontend_object_id,
            "functionDeclaration": "function() { return this.textContent.trim(); }"
        }
    }))
    .await;
    let frontend_text = take_response_by_id(&mut ctx, 5_2);
    assert_eq!(
        frontend_text["result"]["result"]["value"],
        json!("child-body")
    );

    let request_raw = json!({
        "id": 6,
        "method": "DOM.requestNode",
        "params": { "objectId": object_id }
    })
    .to_string();
    let request_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&request_raw)
        .expect("DOM.requestNode with child-frame objectId should enter command task path");
    let request_messages = complete_pending_command_task_for_test(&mut ctx, request_pending).await;
    let request_node = request_messages
        .iter()
        .find(|message| message["id"] == json!(6))
        .unwrap_or_else(|| {
            panic!("pending child-frame DOM.requestNode should produce a response: {request_messages:?}")
        });
    assert!(
        request_node["result"]["nodeId"]
            .as_u64()
            .is_some_and(|id| id > 0),
        "child-frame objectId should resolve to a frontend node id through explicit pending phases: {request_node}"
    );

    let outer_raw = json!({
        "id": 7,
        "method": "DOM.getOuterHTML",
        "params": { "objectId": object_id }
    })
    .to_string();
    let outer_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&outer_raw)
        .expect("DOM.getOuterHTML with child-frame objectId should enter command task path");
    let outer_messages = complete_pending_command_task_for_test(&mut ctx, outer_pending).await;
    let outer_response = outer_messages
        .iter()
        .find(|message| message["id"] == json!(7))
        .unwrap_or_else(|| {
            panic!(
                "pending child-frame DOM.getOuterHTML should produce a response: {outer_messages:?}"
            )
        });
    assert_eq!(
        outer_response["result"]["outerHTML"],
        json!(
            "<body><div id=\"child\" style=\"left:3px;top:4px;width:9px;height:11px\">child-body</div></body>"
        )
    );

    let scroll_raw = json!({
        "id": 8,
        "method": "DOM.scrollIntoViewIfNeeded",
        "params": { "objectId": object_id }
    })
    .to_string();
    let scroll_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&scroll_raw)
        .expect(
            "DOM.scrollIntoViewIfNeeded with child-frame objectId should enter command task path",
        );
    let scroll_messages = complete_pending_command_task_for_test(&mut ctx, scroll_pending).await;
    let scroll_response = scroll_messages
        .iter()
        .find(|message| message["id"] == json!(8))
        .unwrap_or_else(|| {
            panic!(
                "pending child-frame DOM.scrollIntoViewIfNeeded should produce a response: {scroll_messages:?}"
            )
        });
    assert_eq!(scroll_response["result"], json!({}));

    ctx.process_async(json!({
        "id": 9,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id.clone(),
            "functionDeclaration": "function() { return this.querySelector('#child'); }"
        }
    }))
    .await;
    let child_lookup = take_response_by_id(&mut ctx, 9);
    let child_element_object_id = child_lookup["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("child element object id: {child_lookup}"));

    let box_raw = json!({
        "id": 10,
        "method": "DOM.getBoxModel",
        "params": { "objectId": child_element_object_id.clone() }
    })
    .to_string();
    let box_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&box_raw)
        .expect("DOM.getBoxModel with child-frame objectId should enter command task path");
    let box_messages = complete_pending_command_task_for_test(&mut ctx, box_pending).await;
    let box_response = box_messages
        .iter()
        .find(|message| message["id"] == json!(10))
        .unwrap_or_else(|| {
            panic!(
                "pending child-frame DOM.getBoxModel should produce a response: {box_messages:?}"
            )
        });
    assert_eq!(
        box_response["result"]["model"]["content"]
            .as_array()
            .map(Vec::len),
        Some(8),
        "child-frame box model should return a CDP quad-shaped payload: {box_response}"
    );

    let quads_raw = json!({
        "id": 11,
        "method": "DOM.getContentQuads",
        "params": { "objectId": child_element_object_id }
    })
    .to_string();
    let quads_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&quads_raw)
        .expect("DOM.getContentQuads with child-frame objectId should enter command task path");
    let quads_messages = complete_pending_command_task_for_test(&mut ctx, quads_pending).await;
    let quads_response = quads_messages
        .iter()
        .find(|message| message["id"] == json!(11))
        .unwrap_or_else(|| {
            panic!(
                "pending child-frame DOM.getContentQuads should produce a response: {quads_messages:?}"
            )
        });
    assert_eq!(
        quads_response["result"]["quads"][0]
            .as_array()
            .map(Vec::len),
        Some(8),
        "child-frame content quads should return a CDP quad-shaped payload: {quads_response}"
    );

    let describe_raw = json!({
        "id": 12,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id, "depth": 1 }
    })
    .to_string();
    let describe_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&describe_raw)
        .expect("DOM.describeNode with child-frame objectId should enter command task path");
    let describe_messages =
        complete_pending_command_task_for_test(&mut ctx, describe_pending).await;
    let describe_response = describe_messages
        .iter()
        .find(|message| message["id"] == json!(12))
        .unwrap_or_else(|| {
            panic!(
                "pending child-frame DOM.describeNode should produce a response: {describe_messages:?}"
            )
        });
    assert_eq!(
        describe_response["result"]["node"]["nodeName"],
        json!("BODY")
    );
    assert_eq!(
        describe_response["result"]["node"]["children"][0]["nodeName"],
        json!("DIV")
    );
}
