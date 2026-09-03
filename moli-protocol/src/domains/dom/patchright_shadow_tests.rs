use crate::conn::BrowserContext;
use crate::testing::TestContext;
use serde_json::{Value, json};

use super::tests::navigate_to_data_html_async;

fn load_bc(ctx: &mut TestContext, bc_id: &str) {
    let mut bc = BrowserContext::new(bc_id.into());
    bc.set_active_target_id("TID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);
}

fn take_response_by_id(ctx: &mut TestContext, id: u64) -> Value {
    ctx.sent
        .iter()
        .position(|message| message["id"] == json!(id))
        .map(|position| ctx.sent.remove(position))
        .expect("expected response with matching id")
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

async fn enable_runtime_and_take_execution_context_id(ctx: &mut TestContext, id: u64) -> i64 {
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

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_with_pierce_surfaces_closed_shadow_root_tree() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>const host=document.getElementById('host');const root=host.attachShadow({mode:'closed'});const span=document.createElement('span');span.setAttribute('data-inside','yes');root.appendChild(span);</script></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = take_response_by_id(&mut ctx, 2)["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    ctx.process_async(json!({"id": 3, "method": "DOM.querySelector", "params": { "nodeId": root_id, "selector": "#host" }})).await;
    let host_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(host_id > 0);

    ctx.process_async(json!({"id": 4, "method": "DOM.describeNode", "params": { "nodeId": host_id, "depth": -1 }})).await;
    let without_pierce = take_response_by_id(&mut ctx, 4);
    assert!(
        without_pierce["result"]["node"]
            .get("shadowRoots")
            .is_none()
    );

    ctx.process_async(json!({"id": 5, "method": "DOM.describeNode", "params": { "nodeId": host_id, "depth": -1, "pierce": true }})).await;
    let with_pierce = take_response_by_id(&mut ctx, 5);
    let shadow_roots = with_pierce["result"]["node"]["shadowRoots"]
        .as_array()
        .expect("shadow roots");
    assert_eq!(shadow_roots.len(), 1);
    assert_eq!(shadow_roots[0]["shadowRootType"], json!("closed"));
    assert_eq!(
        shadow_roots[0]["children"][0]["attributes"],
        json!(["data-inside", "yes"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_with_pierce_surfaces_closed_shadow_root_tree() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>const host=document.getElementById('host');const root=host.attachShadow({mode:'closed'});const span=document.createElement('span');span.setAttribute('data-inside','yes');root.appendChild(span);</script></body></html>",
    )
    .await;

    ctx.process_async(
        json!({"id": 2, "method": "DOM.getDocument", "params": { "depth": -1, "pierce": true }}),
    )
    .await;
    let document = ctx.take_one();
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let host = &body["children"][0];
    let shadow_roots = host["shadowRoots"].as_array().expect("shadow roots");
    assert_eq!(shadow_roots.len(), 1);
    assert_eq!(shadow_roots[0]["shadowRootType"], json!("closed"));
    assert_eq!(
        shadow_roots[0]["children"][0]["attributes"],
        json!(["data-inside", "yes"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_supports_closed_shadow_root_backend_node_id() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>const host=document.getElementById('host');const root=host.attachShadow({mode:'closed'});const span=document.createElement('span');span.textContent='ok';root.appendChild(span);</script></body></html>",
    )
    .await;
    let execution_context_id = enable_runtime_and_take_execution_context_id(&mut ctx, 10).await;
    let _ = ctx.take_all();
    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = take_response_by_id(&mut ctx, 2)["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    ctx.process_async(json!({"id": 3, "method": "DOM.querySelector", "params": { "nodeId": root_id, "selector": "#host" }})).await;
    let host_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    ctx.process_async(json!({"id": 4, "method": "DOM.describeNode", "params": { "nodeId": host_id, "depth": -1, "pierce": true }})).await;
    let described = take_response_by_id(&mut ctx, 4);
    let shadow_root_backend_id = described["result"]["node"]["shadowRoots"][0]["backendNodeId"]
        .as_u64()
        .unwrap_or(0);
    assert!(shadow_root_backend_id > 0);
    ctx.process_async(json!({"id": 5, "method": "DOM.resolveNode", "params": { "backendNodeId": shadow_root_backend_id, "executionContextId": execution_context_id }})).await;
    let object_id = take_response_by_id(&mut ctx, 5)["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!object_id.is_empty());
    ctx.process_async(json!({"id": 6, "method": "Runtime.callFunctionOn", "params": { "objectId": object_id, "functionDeclaration": "function() { return { mode: this.mode, hostId: this.host ? this.host.id : null }; }", "returnByValue": true }})).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 6)["result"]["result"]["value"],
        json!({ "mode": "closed", "hostId": "host" })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_context_id_alias_supports_patchright_xpath_engine_on_closed_shadow_root() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<section><div id=\"a\"></div><span><div id=\"b\"></div></span></section>';</script></body></html>",
    )
    .await;
    let _ = enable_runtime_and_take_execution_context_id(&mut ctx, 2).await;
    let _ = ctx.take_all();
    ctx.process_async(json!({"id": 3, "method": "Page.createIsolatedWorld", "params": { "frameId": "TID-1", "worldName": "patchright-utility" }})).await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
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
    ctx.process_async(json!({"id": 5, "method": "DOM.resolveNode", "params": { "backendNodeId": shadow_root_backend_id, "contextId": isolated_context_id }})).await;
    let object_id = take_response_by_id(&mut ctx, 5)["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    ctx.process_async(json!({
            "id": 6, "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "arguments": [{ "value": "//div" }],
                "returnByValue": true,
                "functionDeclaration": r#"function(selector) { if (this.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return []; const result = []; const parser = new DOMParser(); function getAllChildElements(node) { const elements = []; const traverse = currentNode => { if (currentNode.nodeType === Node.ELEMENT_NODE) elements.push(currentNode); currentNode.childNodes?.forEach(traverse); }; if (node.nodeType === Node.DOCUMENT_FRAGMENT_NODE || node.nodeType === Node.ELEMENT_NODE) traverse(node); return elements; } const csrHTMLContent = this.innerHTML; const csrChildElements = getAllChildElements(this); const htmlDoc = parser.parseFromString(csrHTMLContent, 'text/html'); const rootDiv = htmlDoc.body; const rootDivChildElements = getAllChildElements(rootDiv); const it = htmlDoc.evaluate(selector, htmlDoc, null, XPathResult.ORDERED_NODE_ITERATOR_TYPE); for (let node = it.iterateNext(); node; node = it.iterateNext()) { const nodeIndex = rootDivChildElements.indexOf(node) - 1; if (nodeIndex >= 0) { const originalNode = csrChildElements[nodeIndex]; if (originalNode.nodeType === Node.ELEMENT_NODE) result.push(originalNode.id); } } return result; }"#
            }
        })).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 6)["result"]["result"]["value"],
        json!(["a", "b"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_frame_selector_flow_recurses_nested_closed_shadow_roots_in_dom_order() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div class='target' id='before'></div><div id='host'></div><div class='target' id='after'></div><script>const outer=document.getElementById('host').attachShadow({mode:'closed'});const nestedHost=document.createElement('section');nestedHost.id='nested-host';outer.innerHTML='<div class=\"target\" id=\"a\"></div>';outer.appendChild(nestedHost);const inner=nestedHost.attachShadow({mode:'closed'});inner.innerHTML='<div class=\"target\" id=\"b\"></div>';</script></body></html>",
    )
    .await;
    let main_context_id = enable_runtime_and_take_execution_context_id(&mut ctx, 2).await;
    let _ = ctx.take_all();
    ctx.process_async(json!({"id": 3, "method": "Page.createIsolatedWorld", "params": { "frameId": "TID-1", "worldName": "patchright-utility" }})).await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    let _ = ctx.take_all();
    ctx.process_async(json!({"id": 4, "method": "Runtime.evaluate", "params": { "expression": "document", "contextId": main_context_id }})).await;
    let document_object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    ctx.process_async(json!({"id": 5, "method": "Runtime.callFunctionOn", "params": { "objectId": document_object_id, "functionDeclaration": "function() { return Array.from(this.querySelectorAll('.target')); }" }})).await;
    let root_array_object_id = take_response_by_id(&mut ctx, 5)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    ctx.process_async(
        json!({"id": 6, "method": "DOM.getDocument", "params": { "pierce": true, "depth": -1 }}),
    )
    .await;
    let described_document = take_response_by_id(&mut ctx, 6)["result"]["root"].clone();
    let closed_shadow_root_backend_ids =
        patchright_collect_closed_shadow_root_backend_ids(&described_document);
    assert_eq!(closed_shadow_root_backend_ids.len(), 2);
    let mut shadow_root_object_ids = Vec::new();
    for backend_node_id in closed_shadow_root_backend_ids {
        ctx.process_async(json!({"id": 7, "method": "DOM.resolveNode", "params": { "backendNodeId": backend_node_id, "contextId": isolated_context_id }})).await;
        shadow_root_object_ids.push(
            take_response_by_id(&mut ctx, 7)["result"]["object"]["objectId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
    }
    let mut selected_object_ids = Vec::new();
    for index in 0..2_u64 {
        ctx.process_async(json!({"id": 8 + index, "method": "Runtime.callFunctionOn", "params": { "objectId": document_object_id, "arguments": [{ "value": index }, { "objectId": root_array_object_id }], "functionDeclaration": "function(i, elems) { return elems[i]; }" }})).await;
        selected_object_ids.push(
            take_response_by_id(&mut ctx, 8 + index)["result"]["result"]["objectId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
    }
    for shadow_root_object_id in shadow_root_object_ids {
        ctx.process_async(json!({"id": 20, "method": "Runtime.callFunctionOn", "params": { "objectId": shadow_root_object_id, "functionDeclaration": "function() { return Array.from(this.querySelectorAll('.target')); }" }})).await;
        let shadow_array_object_id =
            take_response_by_id(&mut ctx, 20)["result"]["result"]["objectId"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
        ctx.process_async(json!({"id": 21, "method": "Runtime.callFunctionOn", "params": { "objectId": shadow_root_object_id, "arguments": [{ "value": 0 }, { "objectId": shadow_array_object_id }], "functionDeclaration": "function(i, elems) { return elems[i]; }" }})).await;
        selected_object_ids.push(
            take_response_by_id(&mut ctx, 21)["result"]["result"]["objectId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
    }
    let mut ordered = Vec::new();
    for (offset, object_id) in selected_object_ids.into_iter().enumerate() {
        let id = 30_u64 + offset as u64;
        ctx.process_async(json!({"id": id, "method": "DOM.describeNode", "params": { "objectId": object_id, "depth": -1 }})).await;
        let described = take_response_by_id(&mut ctx, id)["result"]["node"].clone();
        let backend_node_id = described["backendNodeId"].as_u64().unwrap_or(0);
        let node_position =
            patchright_dom_position_for_backend_node_id(&described_document, backend_node_id)
                .unwrap_or_default();
        ordered.push((node_position, patchright_element_id_attr(&described)));
    }
    ordered.sort_by(|left, right| {
        patchright_position_sort_key(&left.0).cmp(&patchright_position_sort_key(&right.0))
    });
    assert_eq!(
        ordered.into_iter().map(|(_, id)| id).collect::<Vec<_>>(),
        vec!["before", "a", "b", "after"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_frame_selector_nth_after_or_and_chain_uses_current_scoping_order() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div class='target' id='before'></div><div class='target' id='host'></div><div class='target' id='after'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<section><div class=\"target\" id=\"a\"></div><span><div class=\"target\" id=\"b\"></div></span></section>';</script></body></html>",
    )
    .await;
    let main_context_id = enable_runtime_and_take_execution_context_id(&mut ctx, 2).await;
    let _ = ctx.take_all();
    ctx.process_async(json!({"id": 3, "method": "Page.createIsolatedWorld", "params": { "frameId": "TID-1", "worldName": "patchright-utility" }})).await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    let _ = ctx.take_all();
    ctx.process_async(json!({"id": 4, "method": "Runtime.evaluate", "params": { "expression": "document", "contextId": main_context_id }})).await;
    let document_object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    ctx.process_async(
        json!({"id": 5, "method": "DOM.getDocument", "params": { "pierce": true, "depth": -1 }}),
    )
    .await;
    let described_document = take_response_by_id(&mut ctx, 5)["result"]["root"].clone();
    let shadow_root_backend_id =
        patchright_collect_closed_shadow_root_backend_ids(&described_document)[0];
    ctx.process_async(json!({"id": 6, "method": "DOM.resolveNode", "params": { "backendNodeId": shadow_root_backend_id, "contextId": isolated_context_id }})).await;
    let shadow_root_object_id = take_response_by_id(&mut ctx, 6)["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    for (id, expr, target) in [
        (
            7_u64,
            "function() { return Array.from(this.querySelectorAll('.target')); }",
            &document_object_id,
        ),
        (
            8_u64,
            "function() { return Array.from(this.querySelectorAll('.target')); }",
            &shadow_root_object_id,
        ),
        (
            9_u64,
            "function() { return Array.from(this.querySelectorAll('#before, #after')); }",
            &document_object_id,
        ),
        (
            10_u64,
            "function() { return Array.from(this.querySelectorAll('#after, #a')); }",
            &document_object_id,
        ),
        (
            11_u64,
            "function() { return Array.from(this.querySelectorAll('#after, #a')); }",
            &shadow_root_object_id,
        ),
    ] {
        ctx.process_async(json!({"id": id, "method": "Runtime.callFunctionOn", "params": { "objectId": target, "functionDeclaration": expr }})).await;
    }
    let current_root_array = take_response_by_id(&mut ctx, 7)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let current_shadow_array = take_response_by_id(&mut ctx, 8)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let orred_root_array = take_response_by_id(&mut ctx, 9)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let anded_root_array = take_response_by_id(&mut ctx, 10)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let anded_shadow_array = take_response_by_id(&mut ctx, 11)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    let mut current_round = Vec::new();
    for index in 0..3_u64 {
        ctx.process_async(json!({"id": 12, "method": "Runtime.callFunctionOn", "params": { "objectId": document_object_id, "arguments": [{ "value": index }, { "objectId": current_root_array }], "functionDeclaration": "function(i, elems) { return elems[i]; }" }})).await;
        current_round.push(
            take_response_by_id(&mut ctx, 12)["result"]["result"]["objectId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
    }
    for index in 0..2_u64 {
        ctx.process_async(json!({"id": 13, "method": "Runtime.callFunctionOn", "params": { "objectId": shadow_root_object_id, "arguments": [{ "value": index }, { "objectId": current_shadow_array }], "functionDeclaration": "function(i, elems) { return elems[i]; }" }})).await;
        current_round.push(
            take_response_by_id(&mut ctx, 13)["result"]["result"]["objectId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
    }
    for index in 0..2_u64 {
        ctx.process_async(json!({"id": 14, "method": "Runtime.callFunctionOn", "params": { "objectId": document_object_id, "arguments": [{ "value": index }, { "objectId": orred_root_array }], "functionDeclaration": "function(i, elems) { return elems[i]; }" }})).await;
        current_round.push(
            take_response_by_id(&mut ctx, 14)["result"]["result"]["objectId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
    }

    let mut ordered = Vec::new();
    for object_id in current_round {
        ctx.process_async(json!({"id": 15, "method": "DOM.describeNode", "params": { "objectId": object_id, "depth": -1 }})).await;
        let described = take_response_by_id(&mut ctx, 15)["result"]["node"].clone();
        let backend_node_id = described["backendNodeId"].as_u64().unwrap_or(0);
        let node_position =
            patchright_dom_position_for_backend_node_id(&described_document, backend_node_id)
                .unwrap_or_default();
        ordered.push((
            backend_node_id,
            node_position,
            patchright_element_id_attr(&described),
        ));
    }
    ordered.sort_by(|left, right| {
        patchright_position_sort_key(&left.1).cmp(&patchright_position_sort_key(&right.1))
    });
    let mut seen = std::collections::HashSet::new();
    ordered.retain(|(backend_node_id, _, _)| seen.insert(*backend_node_id));
    let deduped_ids = ordered
        .iter()
        .map(|(_, _, id)| id.clone())
        .collect::<Vec<_>>();
    assert_eq!(deduped_ids, vec!["before", "host", "a", "b", "after"]);

    let mut anded_backend_node_ids = std::collections::HashSet::new();
    for (id, array_object_id) in [(16_u64, &anded_root_array), (17_u64, &anded_shadow_array)] {
        ctx.process_async(json!({"id": id, "method": "Runtime.callFunctionOn", "params": { "objectId": array_object_id, "arguments": [{ "value": 0 }, { "objectId": array_object_id }], "functionDeclaration": "function(i, elems) { return elems[i]; }" }})).await;
        let object_id = take_response_by_id(&mut ctx, id)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        ctx.process_async(json!({"id": id + 10, "method": "DOM.describeNode", "params": { "objectId": object_id, "depth": -1 }})).await;
        let described = take_response_by_id(&mut ctx, id + 10)["result"]["node"].clone();
        anded_backend_node_ids.insert(described["backendNodeId"].as_u64().unwrap_or(0));
    }
    let intersected_ids = ordered
        .iter()
        .filter(|(backend_node_id, _, _)| anded_backend_node_ids.contains(backend_node_id))
        .map(|(_, _, id)| id.clone())
        .collect::<Vec<_>>();
    assert_eq!(intersected_ids, vec!["a", "after"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_node_accepts_isolated_execution_context_without_runtime_enable() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>document.getElementById('host').attachShadow({mode:'closed'});</script></body></html>",
    )
    .await;
    ctx.process_async(json!({"id": 2, "method": "Page.createIsolatedWorld", "params": { "frameId": "TID-1", "worldName": "patchright-utility" }})).await;
    let isolated_context_id = take_response_by_id(&mut ctx, 2)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    let _ = ctx.take_all();
    ctx.process_async(json!({"id": 3, "method": "DOM.getDocument"}))
        .await;
    let root_id = take_response_by_id(&mut ctx, 3)["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    ctx.process_async(json!({"id": 4, "method": "DOM.querySelector", "params": { "nodeId": root_id, "selector": "#host" }})).await;
    let host_id = take_response_by_id(&mut ctx, 4)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    ctx.process_async(json!({"id": 5, "method": "DOM.describeNode", "params": { "nodeId": host_id, "depth": -1 }})).await;
    let backend_node_id = take_response_by_id(&mut ctx, 5)["result"]["node"]["backendNodeId"]
        .as_u64()
        .unwrap_or(0);
    ctx.process_async(json!({"id": 6, "method": "DOM.resolveNode", "params": { "backendNodeId": backend_node_id, "executionContextId": isolated_context_id }})).await;
    let resolved = take_response_by_id(&mut ctx, 6);
    assert_eq!(resolved["result"]["object"]["subtype"], json!("node"));
    assert_eq!(
        resolved["result"]["object"]["className"],
        json!("HTMLDivElement")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn request_child_nodes_with_pierce_surfaces_shadow_roots_on_hosts() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='host'></div><script>const host=document.getElementById('host');const root=host.attachShadow({mode:'closed'});const span=document.createElement('span');span.setAttribute('data-inside','yes');root.appendChild(span);</script></body></html>",
    )
    .await;
    ctx.process_async(json!({"id": 2, "method": "DOM.getDocument"}))
        .await;
    let root_id = take_response_by_id(&mut ctx, 2)["result"]["root"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    ctx.process_async(json!({"id": 3, "method": "DOM.querySelector", "params": { "nodeId": root_id, "selector": "body" }})).await;
    let body_id = take_response_by_id(&mut ctx, 3)["result"]["nodeId"]
        .as_u64()
        .unwrap_or(0);
    ctx.process_async(json!({"id": 4, "method": "DOM.requestChildNodes", "params": { "nodeId": body_id, "depth": -1, "pierce": true }})).await;
    let messages = ctx.take_all();
    let event = messages
        .iter()
        .find(|message| {
            message["method"] == json!("DOM.setChildNodes")
                && message["params"]["parentId"] == json!(body_id)
        })
        .expect("DOM.requestChildNodes should emit DOM.setChildNodes");
    assert_eq!(event["method"], json!("DOM.setChildNodes"));
    assert_eq!(event["params"]["parentId"], json!(body_id));
    let nodes = event["params"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let host = nodes
        .iter()
        .find(|node| {
            node["nodeName"] == json!("DIV") && node["attributes"] == json!(["id", "host"])
        })
        .cloned()
        .expect("host node should be present");
    let shadow_roots = host["shadowRoots"].as_array().cloned().unwrap_or_default();
    assert_eq!(shadow_roots.len(), 1);
    assert_eq!(shadow_roots[0]["shadowRootType"], json!("closed"));
    assert_eq!(
        shadow_roots[0]["children"][0]["attributes"],
        json!(["data-inside", "yes"])
    );
    assert!(
        messages
            .iter()
            .any(|message| { message["id"] == json!(4) && message["result"] == json!({}) })
    );
}
