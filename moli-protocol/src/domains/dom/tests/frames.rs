use super::*;
use crate::devtools_runtime::{
    DevToolsCallFunctionCommand, DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
    DevToolsDescribeNodeCommand, DevToolsDomGeometryCommand, DevToolsDomGeometryOperation,
    DevToolsDomNodeReference, DevToolsGetAttributesCommand, DevToolsGetOuterHtmlCommand,
    DevToolsGetPropertyCommand, DevToolsGetTextCommand, DevToolsProtocol,
    DevToolsQuerySelectorCommand, DevToolsRemoteHandleId, DevToolsResolveNodeCommand,
    DevToolsResultOwnership, DevToolsScriptResult, DevToolsScrollIntoViewIfNeededCommand,
    DevToolsSessionId, DevToolsTargetId,
};

#[tokio::test(flavor = "multi_thread")]
async fn get_document_preserves_html_doctype_name_case() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(&mut ctx, 25, "<!DOCTYPE HTML><html><body></body></html>").await;

    ctx.process_async(json!({
        "id": 26,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 26);
    let doctype = response["result"]["root"]["children"]
        .as_array()
        .and_then(|children| children.iter().find(|node| node["nodeType"] == json!(10)))
        .expect("DOM.getDocument should expose the parsed doctype");

    assert_eq!(doctype["nodeName"], json!("html"));
    assert_eq!(doctype["name"], json!("html"));
    assert_eq!(doctype["localName"], json!(""));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_preserves_namespace_qualified_element_node_names() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        251,
        "<!doctype html><html><body>\
         <div id='html'></div>\
         <svg id='svg'><linearGradient id='gradient'></linearGradient></svg>\
         <math id='math'><mi id='mi'>x</mi></math>\
         </body></html>",
    )
    .await;
    ctx.process_async(json!({
        "id": 252,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "\
                const prefixedSvg = document.createElementNS('http://www.w3.org/2000/svg', 'svg:g');\
                prefixedSvg.id = 'prefixed-svg';\
                document.body.append(prefixedSvg);\
                const foreign = document.createElementNS('urn:moli:test', 'lm:node');\
                foreign.id = 'foreign';\
                document.body.append(foreign);"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 252);
    ctx.process_async(json!({
        "id": 253,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 253);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");

    for (id, node_name, local_name) in [
        ("html", "DIV", "div"),
        ("svg", "svg", "svg"),
        ("gradient", "linearGradient", "linearGradient"),
        ("math", "math", "math"),
        ("mi", "mi", "mi"),
        ("prefixed-svg", "svg:g", "g"),
        ("foreign", "lm:node", "node"),
    ] {
        let node = node_tree_element_by_attribute(body, "id", id)
            .unwrap_or_else(|| panic!("{id} element"));
        assert_eq!(
            node["nodeName"],
            json!(node_name),
            "{id} CDP nodeName should preserve its namespace naming rules: {node}"
        );
        assert_eq!(
            node["localName"],
            json!(local_name),
            "{id} CDP localName should remain prefix-free: {node}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_shallow_template_content_fragment() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        41,
        "<!doctype html><html><body>\
         <template id='empty'></template>\
         <template id='full'><article><span>inside</span></article><!--tail--></template>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 42,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": false }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 42);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let empty =
        node_array_element_by_attribute(&body["children"], "id", "empty").expect("empty template");
    let full = node_array_element_by_attribute(&body["children"], "id", "full")
        .expect("populated template");

    assert_eq!(empty["childNodeCount"], json!(0));
    assert_eq!(empty["templateContent"]["nodeType"], json!(11));
    assert_eq!(
        empty["templateContent"]["nodeName"],
        json!("#document-fragment")
    );
    assert_eq!(empty["templateContent"]["childNodeCount"], json!(0));

    let template_content = &full["templateContent"];
    assert_eq!(full["childNodeCount"], json!(0));
    assert_eq!(template_content["nodeType"], json!(11));
    assert_eq!(template_content["nodeName"], json!("#document-fragment"));
    assert_eq!(template_content["localName"], json!(""));
    assert_eq!(template_content["nodeValue"], json!(""));
    assert_eq!(template_content["childNodeCount"], json!(2));
    assert!(
        template_content.get("parentId").is_none(),
        "template content is an associated fragment, not a template child: {template_content}"
    );
    assert!(
        template_content.get("children").is_none(),
        "a template host projects only the shallow content fragment: {template_content}"
    );

    let template_content_node_id = template_content["nodeId"]
        .as_u64()
        .expect("template content frontend node id");
    let template_node_id = full["nodeId"].as_u64().expect("template frontend node id");
    let template_content_backend_node_id = template_content["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("template content backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(template_content_backend_node_id),
        "template content backend id should be renderer-owned: {template_content}"
    );

    ctx.process_async(json!({
        "id": 43,
        "method": "DOM.describeNode",
        "params": {
            "nodeId": template_node_id,
            "depth": 0,
            "pierce": false
        }
    }))
    .await;
    let described_template = take_response_by_id(&mut ctx, 43);
    assert_eq!(
        described_template["result"]["node"]["templateContent"]["nodeId"],
        json!(template_content_node_id)
    );
    assert!(
        described_template["result"]["node"]
            .get("children")
            .is_none(),
        "depth zero should not expand ordinary template children: {described_template}"
    );

    ctx.process_async(json!({
        "id": 44,
        "method": "DOM.describeNode",
        "params": {
            "nodeId": template_content_node_id,
            "depth": -1,
            "pierce": false
        }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 44);
    let fragment = &described["result"]["node"];
    let article = child_element_by_node_name(fragment, "ARTICLE");
    let span = child_element_by_node_name(article, "SPAN");
    assert_eq!(span["children"][0]["nodeName"], json!("#text"));
    assert_eq!(span["children"][0]["nodeValue"], json!("inside"));
    assert_eq!(fragment["children"][1]["nodeName"], json!("#comment"));
    assert_eq!(fragment["children"][1]["nodeValue"], json!("tail"));

    ctx.process_async(json!({
        "id": 45,
        "method": "DOM.getFlattenedDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let flattened = take_response_by_id(&mut ctx, 45);
    let flattened_nodes = flattened["result"]["nodes"]
        .as_array()
        .expect("flattened nodes");
    let flattened_template =
        node_array_element_by_attribute(&flattened["result"]["nodes"], "id", "full")
            .expect("flattened template");
    assert!(
        flattened_template.get("templateContent").is_none(),
        "Chromium does not nest templateContent in flattened nodes: {flattened_template}"
    );
    assert!(
        flattened_nodes
            .iter()
            .all(|node| node["nodeType"] != json!(11)),
        "Chromium does not add template fragments to the flattened node array: {flattened}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_generated_marker_pseudo_elements() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        46,
        "<!doctype html><html><head><style>\
         .normal,.none,.content { display: list-item; }\
         .none,.content { list-style: none; }\
         .content::marker { content: 'custom'; }\
         .image { display: list-item; list-style: none url(data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==); }\
         .inline { display: inline list-item; }\
         .content-none { display: list-item; }\
         .content-none::marker { content: none; }\
         </style></head><body>\
         <div id='normal' class='normal'>normal</div>\
         <div id='none' class='none'>none</div>\
         <div id='content' class='content'>content</div>\
         <div id='image' class='image'>image</div>\
         <div id='inline' class='inline'>inline</div>\
         <div id='content-none' class='content-none'>content none</div>\
         <li id='li'>default list item</li>\
         <div id='block'>block</div>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 47,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": false }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 47);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let normal =
        node_array_element_by_attribute(&body["children"], "id", "normal").expect("normal host");
    let none = node_array_element_by_attribute(&body["children"], "id", "none").expect("none host");
    let content =
        node_array_element_by_attribute(&body["children"], "id", "content").expect("content host");
    let block =
        node_array_element_by_attribute(&body["children"], "id", "block").expect("block host");

    let normal_marker = &normal["pseudoElements"][0];
    assert_eq!(normal_marker["nodeType"], json!(1));
    assert_eq!(normal_marker["nodeName"], json!("::marker"));
    assert_eq!(normal_marker["localName"], json!("::marker"));
    assert_eq!(normal_marker["nodeValue"], json!(""));
    assert_eq!(normal_marker["childNodeCount"], json!(0));
    assert_eq!(normal_marker["attributes"], json!([]));
    assert_eq!(normal_marker["pseudoType"], json!("marker"));
    assert!(
        normal_marker.get("parentId").is_none(),
        "a pseudo element is associated through pseudoElements, not an ordinary parent edge"
    );
    assert_ne!(normal_marker["nodeId"], normal["nodeId"]);
    assert_ne!(normal_marker["backendNodeId"], normal["backendNodeId"]);
    assert!(
        is_renderer_backend_node_id(
            normal_marker["backendNodeId"]
                .as_u64()
                .and_then(|id| u32::try_from(id).ok())
                .expect("marker backend node id")
        ),
        "marker should use the renderer backend id namespace: {normal_marker}"
    );

    assert!(
        none.get("pseudoElements").is_none(),
        "list-style:none without explicit marker content suppresses the marker: {none}"
    );
    assert_eq!(content["pseudoElements"][0]["pseudoType"], json!("marker"));
    for id in ["image", "inline", "li"] {
        let host = node_array_element_by_attribute(&body["children"], "id", id)
            .unwrap_or_else(|| panic!("{id} host"));
        assert_eq!(
            host["pseudoElements"][0]["pseudoType"],
            json!("marker"),
            "{id} should generate ::marker: {host}"
        );
    }
    let content_none = node_array_element_by_attribute(&body["children"], "id", "content-none")
        .expect("content-none host");
    assert!(
        content_none.get("pseudoElements").is_none(),
        "content:none suppresses the marker even when list style is otherwise visible: {content_none}"
    );
    assert!(
        block.get("pseudoElements").is_none(),
        "a non-list-item must not expose ::marker: {block}"
    );

    let normal_node_id = normal["nodeId"].as_u64().expect("normal frontend id");
    let marker_backend_node_id = normal_marker["backendNodeId"]
        .as_u64()
        .expect("marker backend id");
    ctx.process_async(json!({
        "id": 48,
        "method": "DOM.describeNode",
        "params": {
            "nodeId": normal_node_id,
            "depth": 0,
            "pierce": false
        }
    }))
    .await;
    let described = take_response_by_id(&mut ctx, 48);
    assert_eq!(
        described["result"]["node"]["pseudoElements"][0]["backendNodeId"],
        json!(marker_backend_node_id),
        "depth zero should retain the associated pseudo element with stable backend identity"
    );

    ctx.process_async(json!({
        "id": 50,
        "method": "DOM.describeNode",
        "params": {
            "backendNodeId": marker_backend_node_id,
            "depth": -1,
            "pierce": true
        }
    }))
    .await;
    let described_marker = take_response_by_id(&mut ctx, 50);
    assert_eq!(described_marker["result"]["node"]["nodeType"], json!(1));
    assert_eq!(
        described_marker["result"]["node"]["nodeName"],
        json!("::marker")
    );
    assert_eq!(
        described_marker["result"]["node"]["localName"],
        json!("::marker")
    );
    assert_eq!(
        described_marker["result"]["node"]["pseudoType"],
        json!("marker")
    );
    assert_eq!(
        described_marker["result"]["node"]["backendNodeId"],
        json!(marker_backend_node_id),
        "direct pseudo-element describe should preserve its independent backend identity"
    );
    assert!(
        described_marker["result"]["node"]
            .get("pseudoElements")
            .is_none(),
        "directly described ::marker is a leaf: {described_marker}"
    );

    ctx.process_async(json!({
        "id": 49,
        "method": "DOM.getFlattenedDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let flattened = take_response_by_id(&mut ctx, 49);
    assert!(
        flattened["result"]["nodes"]
            .as_array()
            .expect("flattened nodes")
            .iter()
            .all(|node| node.get("pseudoType").is_none()),
        "Chromium does not add pseudo elements to DOM.getFlattenedDocument: {flattened}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_suppresses_markers_below_display_none_ancestors() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        501,
        "<!doctype html><html><body>\
         <ul><li id='visible'>visible</li></ul>\
         <div hidden><ul><li id='hidden'>hidden</li></ul></div>\
         <div style='display:none'><ul><li id='display-none'>display none</li></ul></div>\
         <div style='visibility:hidden'><ul><li id='visibility'>visibility</li></ul></div>\
         <div style='content-visibility:hidden'><ul><li id='content-visibility'>content visibility</li></ul></div>\
         <div id='dynamic-parent'><ul><li id='dynamic'>dynamic</li></ul></div>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 502,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 502);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    for id in ["visible", "visibility", "content-visibility", "dynamic"] {
        let host =
            node_tree_element_by_attribute(body, "id", id).unwrap_or_else(|| panic!("{id} host"));
        assert_eq!(
            host["pseudoElements"][0]["pseudoType"],
            json!("marker"),
            "{id} should retain its marker: {host}"
        );
    }
    for id in ["hidden", "display-none"] {
        let host =
            node_tree_element_by_attribute(body, "id", id).unwrap_or_else(|| panic!("{id} host"));
        assert!(
            host.get("pseudoElements").is_none(),
            "an ancestor with display:none must suppress {id}'s marker: {host}"
        );
    }

    ctx.process_async(json!({
        "id": 503,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('dynamic-parent').hidden=true" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 503);
    ctx.process_async(json!({
        "id": 504,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let hidden = take_response_by_id(&mut ctx, 504);
    let html = child_element_by_node_name(&hidden["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let dynamic =
        node_tree_element_by_attribute(body, "id", "dynamic").expect("hidden dynamic host");
    assert!(
        dynamic.get("pseudoElements").is_none(),
        "dynamically hiding an ancestor must remove the descendant marker: {dynamic}"
    );

    ctx.process_async(json!({
        "id": 505,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('dynamic-parent').hidden=false" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 505);
    ctx.process_async(json!({
        "id": 506,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let shown = take_response_by_id(&mut ctx, 506);
    let html = child_element_by_node_name(&shown["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let dynamic =
        node_tree_element_by_attribute(body, "id", "dynamic").expect("shown dynamic host");
    assert_eq!(
        dynamic["pseudoElements"][0]["pseudoType"],
        json!("marker"),
        "showing the ancestor again must restore the descendant marker: {dynamic}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_text_control_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        51,
        "<!doctype html><html><body>\
         <input id='input' value='alpha'>\
         <input id='empty'>\
         <input id='email' type='email' value='user@example.test'>\
         <textarea id='textarea'>beta</textarea>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 52,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 52);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let input =
        node_array_element_by_attribute(&body["children"], "id", "input").expect("input host");
    let empty =
        node_array_element_by_attribute(&body["children"], "id", "empty").expect("empty host");
    let email =
        node_array_element_by_attribute(&body["children"], "id", "email").expect("email host");
    let textarea = node_array_element_by_attribute(&body["children"], "id", "textarea")
        .expect("textarea host");

    let input_root = &input["shadowRoots"][0];
    assert_eq!(input_root["nodeType"], json!(11));
    assert_eq!(input_root["nodeName"], json!("#document-fragment"));
    assert_eq!(input_root["localName"], json!(""));
    assert_eq!(input_root["nodeValue"], json!(""));
    assert_eq!(input_root["childNodeCount"], json!(1));
    assert_eq!(input_root["shadowRootType"], json!("user-agent"));
    assert!(
        input_root.get("parentId").is_none(),
        "UA shadow root is associated through shadowRoots: {input_root}"
    );
    let input_editor = &input_root["children"][0];
    assert_eq!(input_editor["nodeName"], json!("DIV"));
    assert_eq!(input_editor["attributes"], json!([]));
    assert_eq!(input_editor["children"][0]["nodeName"], json!("#text"));
    assert_eq!(input_editor["children"][0]["nodeValue"], json!("alpha"));
    assert_ne!(input_root["backendNodeId"], input["backendNodeId"]);
    assert_ne!(input_editor["backendNodeId"], input_root["backendNodeId"]);

    assert_eq!(
        empty["shadowRoots"][0]["children"][0]["childNodeCount"],
        json!(0)
    );
    assert_eq!(
        email["shadowRoots"][0]["children"][0]["children"][0]["nodeValue"],
        json!("user@example.test")
    );
    assert_eq!(
        textarea["shadowRoots"][0]["children"][0]["children"][0]["nodeValue"],
        json!("beta")
    );
    assert_ne!(
        textarea["shadowRoots"][0]["backendNodeId"], input_root["backendNodeId"],
        "different hosts must not share generated-node identity"
    );

    let input_backend_node_id = input["backendNodeId"].as_u64().expect("input backend id");
    let input_root_backend_node_id = input_root["backendNodeId"]
        .as_u64()
        .expect("input shadow root backend id");
    let original_text_backend_node_id = input_editor["children"][0]["backendNodeId"]
        .as_u64()
        .expect("input text backend id");

    ctx.process_async(json!({
        "id": 53,
        "method": "DOM.describeNode",
        "params": {
            "backendNodeId": input_backend_node_id,
            "depth": 0,
            "pierce": false
        }
    }))
    .await;
    let shallow_host = take_response_by_id(&mut ctx, 53);
    let shallow_root = &shallow_host["result"]["node"]["shadowRoots"][0];
    assert_eq!(shallow_root["childNodeCount"], json!(1));
    assert!(
        shallow_root.get("children").is_none(),
        "host depth zero retains only the shallow UA shadow association: {shallow_host}"
    );

    ctx.process_async(json!({
        "id": 54,
        "method": "DOM.describeNode",
        "params": {
            "backendNodeId": input_root_backend_node_id,
            "depth": -1,
            "pierce": false
        }
    }))
    .await;
    let described_root = take_response_by_id(&mut ctx, 54);
    assert_eq!(
        described_root["result"]["node"]["children"][0]["children"][0]["nodeValue"],
        json!("alpha"),
        "directly describing a UA shadow node expands its ordinary descendants without pierce"
    );

    ctx.process_async(json!({
        "id": 55,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('input').value='changed'; document.getElementById('textarea').value=''"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 55);
    ctx.process_async(json!({
        "id": 56,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 56);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_input =
        node_array_element_by_attribute(&body["children"], "id", "input").expect("updated input");
    let updated_textarea = node_array_element_by_attribute(&body["children"], "id", "textarea")
        .expect("updated textarea");
    let updated_input_root = &updated_input["shadowRoots"][0];
    assert_eq!(
        updated_input_root["children"][0]["children"][0]["nodeValue"],
        json!("changed")
    );
    assert_eq!(
        updated_input_root["backendNodeId"],
        json!(input_root_backend_node_id),
        "the UA root survives value-only updates"
    );
    assert_ne!(
        updated_input_root["children"][0]["children"][0]["backendNodeId"],
        json!(original_text_backend_node_id),
        "Chromium recreates the internal text node when its value changes"
    );
    assert_eq!(
        updated_textarea["shadowRoots"][0]["children"][0]["childNodeCount"],
        json!(0)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_include_whitespace_projects_text_control_user_agent_shadow_text() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        57,
        "<!doctype html><html><body><input id='whitespace' value='   '></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 58,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let default_document = take_response_by_id(&mut ctx, 58);
    let html = child_element_by_node_name(&default_document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let input = node_array_element_by_attribute(&body["children"], "id", "whitespace")
        .expect("whitespace input host");
    let default_editor = &input["shadowRoots"][0]["children"][0];
    assert_eq!(default_editor["childNodeCount"], json!(0));
    assert!(
        default_editor.get("children").is_none(),
        "default DOM projection must omit UA shadow whitespace text: {default_editor}"
    );

    ctx.process_async(json!({ "id": 59, "method": "DOM.disable" }))
        .await;
    ctx.expect_result(59, json!({}), None);
    ctx.process_async(json!({
        "id": 60,
        "method": "DOM.enable",
        "params": { "includeWhitespace": "all" }
    }))
    .await;
    ctx.expect_result(60, json!({}), None);
    ctx.process_async(json!({
        "id": 61,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let all_document = take_response_by_id(&mut ctx, 61);
    let html = child_element_by_node_name(&all_document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let input = node_array_element_by_attribute(&body["children"], "id", "whitespace")
        .expect("whitespace input host");
    let all_editor = &input["shadowRoots"][0]["children"][0];
    assert_eq!(all_editor["childNodeCount"], json!(1));
    assert_eq!(all_editor["children"][0]["nodeName"], json!("#text"));
    assert_eq!(all_editor["children"][0]["nodeValue"], json!("   "));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_multiline_textarea_user_agent_editor_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        561,
        "<!doctype html><html><body>\
         <textarea id='between'></textarea>\
         <textarea id='leading'></textarea>\
         <textarea id='trailing'></textarea>\
         <textarea id='blank-line'></textarea>\
         <textarea id='crlf'></textarea>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 562,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "\
    document.getElementById('between').value='line one\\nline two';\
    document.getElementById('leading').value='\\nalpha';\
    document.getElementById('trailing').value='omega\\n';\
    document.getElementById('blank-line').value='a\\n\\nb';\
    document.getElementById('crlf').value='left\\r\\nright';"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 562);

    ctx.process_async(json!({
        "id": 563,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 563);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let editor_child_shape = |id: &str| {
        let textarea = node_array_element_by_attribute(&body["children"], "id", id)
            .unwrap_or_else(|| panic!("textarea {id}"));
        textarea["shadowRoots"][0]["children"][0]["children"]
            .as_array()
            .expect("textarea editor children")
            .iter()
            .map(|node| {
                (
                    node["nodeName"]
                        .as_str()
                        .expect("generated node name")
                        .to_owned(),
                    node["nodeValue"]
                        .as_str()
                        .expect("generated node value")
                        .to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        editor_child_shape("between"),
        vec![
            ("#text".to_owned(), "line one".to_owned()),
            ("BR".to_owned(), String::new()),
            ("#text".to_owned(), "line two".to_owned()),
        ]
    );
    assert_eq!(
        editor_child_shape("leading"),
        vec![
            ("BR".to_owned(), String::new()),
            ("#text".to_owned(), "alpha".to_owned()),
        ]
    );
    assert_eq!(
        editor_child_shape("trailing"),
        vec![
            ("#text".to_owned(), "omega".to_owned()),
            ("BR".to_owned(), String::new()),
            ("BR".to_owned(), String::new()),
        ]
    );
    assert_eq!(
        editor_child_shape("blank-line"),
        vec![
            ("#text".to_owned(), "a".to_owned()),
            ("BR".to_owned(), String::new()),
            ("BR".to_owned(), String::new()),
            ("#text".to_owned(), "b".to_owned()),
        ]
    );
    assert_eq!(
        editor_child_shape("crlf"),
        vec![
            ("#text".to_owned(), "left".to_owned()),
            ("BR".to_owned(), String::new()),
            ("#text".to_owned(), "right".to_owned()),
        ],
        "the textarea API normalizes CRLF before projecting the editor tree"
    );

    let between = node_array_element_by_attribute(&body["children"], "id", "between")
        .expect("between textarea");
    let between_root_backend = between["shadowRoots"][0]["backendNodeId"].clone();
    let between_editor_backend = between["shadowRoots"][0]["children"][0]["backendNodeId"].clone();
    let original_child_backends = between["shadowRoots"][0]["children"][0]["children"]
        .as_array()
        .expect("between editor children")
        .iter()
        .map(|node| {
            node["backendNodeId"]
                .as_u64()
                .expect("generated child backend id")
        })
        .collect::<Vec<_>>();
    let mut unique_child_backends = original_child_backends.clone();
    unique_child_backends.sort_unstable();
    unique_child_backends.dedup();
    assert_eq!(
        unique_child_backends.len(),
        original_child_backends.len(),
        "each generated textarea editor child needs an independent backend identity"
    );

    ctx.process_async(json!({
        "id": 564,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('between').value='line one\\nchanged'"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 564);
    ctx.process_async(json!({
        "id": 565,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 565);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_between = node_array_element_by_attribute(&body["children"], "id", "between")
        .expect("updated between textarea");
    let updated_editor = &updated_between["shadowRoots"][0]["children"][0];
    assert_eq!(
        updated_between["shadowRoots"][0]["backendNodeId"], between_root_backend,
        "textarea UA root survives value-only updates"
    );
    assert_eq!(
        updated_editor["backendNodeId"], between_editor_backend,
        "textarea editor survives value-only updates"
    );
    assert_eq!(
        updated_editor["children"]
            .as_array()
            .expect("updated editor children")
            .iter()
            .map(|node| (
                node["nodeName"].as_str().expect("node name").to_owned(),
                node["nodeValue"].as_str().expect("node value").to_owned(),
            ))
            .collect::<Vec<_>>(),
        vec![
            ("#text".to_owned(), "line one".to_owned()),
            ("BR".to_owned(), String::new()),
            ("#text".to_owned(), "changed".to_owned()),
        ]
    );
    let updated_child_backends = updated_editor["children"]
        .as_array()
        .expect("updated editor children")
        .iter()
        .map(|node| {
            node["backendNodeId"]
                .as_u64()
                .expect("updated child backend id")
        })
        .collect::<Vec<_>>();
    assert!(
        original_child_backends
            .iter()
            .all(|backend| !updated_child_backends.contains(backend)),
        "Chromium recreates all textarea editor leaves when the value changes"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_select_and_option_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        57,
        "<!doctype html><html><body>\
         <select id='single'>\
           <option id='alpha' value='a'>Alpha</option>\
           <option id='bee' label='Bee' value='b'>Body B</option>\
           <option id='empty'></option>\
         </select>\
         <select id='multiple' multiple><option>One</option></select>\
         <select id='sized' size='2'><option>Two</option></select>\
         <select id='no-options'></select>\
         <select id='empty-selected'><option></option></select>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 58,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 58);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let single =
        node_array_element_by_attribute(&body["children"], "id", "single").expect("single select");
    let multiple = node_array_element_by_attribute(&body["children"], "id", "multiple")
        .expect("multiple select");
    let sized =
        node_array_element_by_attribute(&body["children"], "id", "sized").expect("sized select");
    let no_options = node_array_element_by_attribute(&body["children"], "id", "no-options")
        .expect("select without options");
    let empty_selected = node_array_element_by_attribute(&body["children"], "id", "empty-selected")
        .expect("select with empty selected option");

    let single_root = &single["shadowRoots"][0];
    assert_eq!(single_root["shadowRootType"], json!("user-agent"));
    assert_eq!(single_root["childNodeCount"], json!(4));
    assert_eq!(single_root["children"][0]["nodeName"], json!("DIV"));
    assert_eq!(
        single_root["children"][0]["attributes"],
        json!([
            "aria-hidden",
            "true",
            "pseudo",
            "-internal-select-inner-element"
        ])
    );
    assert_eq!(
        single_root["children"][0]["children"][0]["nodeValue"],
        json!("Alpha")
    );
    assert_eq!(single_root["children"][1]["nodeName"], json!("SLOT"));
    assert_eq!(
        single_root["children"][1]["attributes"],
        json!(["pseudo", "-internal-select-button-slot"])
    );
    assert_eq!(
        single_root["children"][2]["attributes"],
        json!(["pseudo", "picker(select)", "popover", "auto"])
    );
    assert_eq!(
        single_root["children"][2]["children"][0]["attributes"],
        json!(["id", "select-popover-options"])
    );
    assert_eq!(
        single_root["children"][3]["attributes"],
        json!([
            "pseudo",
            "-internal-select-autofill-preview",
            "popover",
            "manual"
        ])
    );
    assert_eq!(
        single_root["children"][3]["children"][0]["attributes"],
        json!(["pseudo", "-internal-select-autofill-preview-text"])
    );

    for list_box in [multiple, sized] {
        let root = &list_box["shadowRoots"][0];
        assert_eq!(root["childNodeCount"], json!(1));
        assert_eq!(root["children"][0]["nodeName"], json!("SLOT"));
        assert_eq!(
            root["children"][0]["attributes"],
            json!(["id", "select-options"])
        );
    }
    for empty_label in [no_options, empty_selected] {
        let inner = &empty_label["shadowRoots"][0]["children"][0];
        assert_eq!(inner["childNodeCount"], json!(0));
        assert!(
            inner.get("children").is_none(),
            "default DOM projection omits an empty UA label text node: {inner}"
        );
    }

    let alpha =
        node_array_element_by_attribute(&single["children"], "id", "alpha").expect("alpha option");
    let bee =
        node_array_element_by_attribute(&single["children"], "id", "bee").expect("bee option");
    let empty =
        node_array_element_by_attribute(&single["children"], "id", "empty").expect("empty option");
    let alpha_root = &alpha["shadowRoots"][0];
    assert_eq!(alpha_root["childNodeCount"], json!(2));
    assert_eq!(alpha_root["children"][0]["nodeName"], json!("SPAN"));
    assert_eq!(
        alpha_root["children"][0]["attributes"],
        json!([
            "pseudo",
            "-internal-option-label-container",
            "aria-hidden",
            "true"
        ])
    );
    assert_eq!(
        alpha_root["children"][0]["children"][0]["nodeValue"],
        json!("Alpha")
    );
    assert_eq!(alpha_root["children"][1]["nodeName"], json!("SLOT"));
    assert_eq!(
        alpha_root["children"][1]["attributes"],
        json!(["pseudo", "-internal-option-slot"])
    );
    assert_eq!(
        bee["shadowRoots"][0]["children"][0]["children"][0]["nodeValue"],
        json!("Bee"),
        "option label attribute is the rendered UA label"
    );
    assert_eq!(
        empty["shadowRoots"][0]["children"][0]["childNodeCount"],
        json!(0)
    );
    assert_ne!(single_root["backendNodeId"], single["backendNodeId"]);
    assert_ne!(
        alpha_root["backendNodeId"], single_root["backendNodeId"],
        "each host owns a disjoint generated tree"
    );

    let select_text_backend_node_id = single_root["children"][0]["children"][0]["backendNodeId"]
        .as_u64()
        .expect("select inner text backend id");
    let bee_label_backend_node_id =
        bee["shadowRoots"][0]["children"][0]["children"][0]["backendNodeId"]
            .as_u64()
            .expect("option label backend id");
    ctx.process_async(json!({
        "id": 59,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('single').selectedIndex=1; document.getElementById('bee').label='Changed'"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 59);
    ctx.process_async(json!({
        "id": 60,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 60);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_single =
        node_array_element_by_attribute(&body["children"], "id", "single").expect("updated select");
    let updated_bee = node_array_element_by_attribute(&updated_single["children"], "id", "bee")
        .expect("updated bee option");
    assert_eq!(
        updated_single["shadowRoots"][0]["children"][0]["children"][0]["nodeValue"],
        json!("Changed")
    );
    assert_eq!(
        updated_single["shadowRoots"][0]["children"][0]["children"][0]["backendNodeId"],
        json!(select_text_backend_node_id),
        "Chromium reuses the select inner text node while changing its data"
    );
    assert_eq!(
        updated_bee["shadowRoots"][0]["children"][0]["children"][0]["nodeValue"],
        json!("Changed")
    );
    assert_ne!(
        updated_bee["shadowRoots"][0]["children"][0]["children"][0]["backendNodeId"],
        json!(bee_label_backend_node_id),
        "Chromium rebuilds the option label text node when label changes"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_search_input_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        601,
        "<!doctype html><html><body>\
         <input id='static-filled' type='search' value='initial'>\
         <input id='static-empty' type='search'>\
         </body></html>",
    )
    .await;
    ctx.process_async(json!({
        "id": 602,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "\
                const search = document.createElement('input');\
                search.id = 'dynamic';\
                search.type = 'search';\
                search.value = 'all';\
                search.setAttribute('value', 'all');\
                document.body.append(search);"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 602);

    ctx.process_async(json!({
        "id": 603,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 603);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let static_filled = node_array_element_by_attribute(&body["children"], "id", "static-filled")
        .expect("static filled search input");
    let static_empty = node_array_element_by_attribute(&body["children"], "id", "static-empty")
        .expect("static empty search input");
    let dynamic = node_array_element_by_attribute(&body["children"], "id", "dynamic")
        .expect("dynamic search input");

    let root = &dynamic["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(1));
    let container = &root["children"][0];
    assert_eq!(
        container["attributes"],
        json!([
            "id",
            "text-field-container",
            "pseudo",
            "-webkit-textfield-decoration-container",
            "style",
            "unicode-bidi: normal;"
        ])
    );
    assert_eq!(container["childNodeCount"], json!(2));
    let viewport = &container["children"][0];
    assert_eq!(viewport["attributes"], json!(["id", "editing-view-port"]));
    let editor = &viewport["children"][0];
    assert_eq!(editor["attributes"], json!([]));
    assert_eq!(editor["children"][0]["nodeValue"], json!("all"));
    let clear = &container["children"][1];
    assert_eq!(
        clear["attributes"],
        json!([
            "pseudo",
            "-webkit-search-cancel-button",
            "id",
            "search-clear",
            "style",
            ""
        ])
    );
    assert_eq!(
        static_filled["shadowRoots"][0]["children"][0]["children"][1]["attributes"],
        json!([
            "pseudo",
            "-webkit-search-cancel-button",
            "id",
            "search-clear"
        ])
    );
    assert_eq!(
        static_empty["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["childNodeCount"],
        json!(0)
    );
    assert_eq!(
        static_empty["shadowRoots"][0]["children"][0]["children"][1]["attributes"],
        json!([
            "pseudo",
            "-webkit-search-cancel-button",
            "id",
            "search-clear",
            "style",
            "opacity: 0; pointer-events: none;"
        ])
    );

    let stable_backend_node_ids = [
        root["backendNodeId"].as_u64().expect("root backend id"),
        container["backendNodeId"]
            .as_u64()
            .expect("container backend id"),
        viewport["backendNodeId"]
            .as_u64()
            .expect("viewport backend id"),
        editor["backendNodeId"].as_u64().expect("editor backend id"),
        clear["backendNodeId"].as_u64().expect("clear backend id"),
    ];
    let text_backend_node_id = editor["children"][0]["backendNodeId"]
        .as_u64()
        .expect("text backend id");
    ctx.process_async(json!({
        "id": 604,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('dynamic').value='next'" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 604);
    ctx.process_async(json!({
        "id": 605,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 605);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_dynamic = node_array_element_by_attribute(&body["children"], "id", "dynamic")
        .expect("updated dynamic search input");
    let updated_root = &updated_dynamic["shadowRoots"][0];
    let updated_container = &updated_root["children"][0];
    let updated_viewport = &updated_container["children"][0];
    let updated_editor = &updated_viewport["children"][0];
    let updated_clear = &updated_container["children"][1];
    assert_eq!(
        [
            updated_root["backendNodeId"].as_u64().unwrap(),
            updated_container["backendNodeId"].as_u64().unwrap(),
            updated_viewport["backendNodeId"].as_u64().unwrap(),
            updated_editor["backendNodeId"].as_u64().unwrap(),
            updated_clear["backendNodeId"].as_u64().unwrap(),
        ],
        stable_backend_node_ids
    );
    assert_eq!(updated_editor["children"][0]["nodeValue"], json!("next"));
    assert_ne!(
        updated_editor["children"][0]["backendNodeId"],
        json!(text_backend_node_id),
        "Chromium rebuilds the search editor text node when value changes"
    );

    ctx.process_async(json!({
        "id": 606,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('dynamic').value=''" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 606);
    ctx.process_async(json!({
        "id": 607,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let cleared = take_response_by_id(&mut ctx, 607);
    let html = child_element_by_node_name(&cleared["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let cleared_dynamic = node_array_element_by_attribute(&body["children"], "id", "dynamic")
        .expect("cleared dynamic search input");
    let cleared_root = &cleared_dynamic["shadowRoots"][0];
    let cleared_container = &cleared_root["children"][0];
    let cleared_viewport = &cleared_container["children"][0];
    let cleared_editor = &cleared_viewport["children"][0];
    let cleared_clear = &cleared_container["children"][1];
    assert_eq!(
        [
            cleared_root["backendNodeId"].as_u64().unwrap(),
            cleared_container["backendNodeId"].as_u64().unwrap(),
            cleared_viewport["backendNodeId"].as_u64().unwrap(),
            cleared_editor["backendNodeId"].as_u64().unwrap(),
            cleared_clear["backendNodeId"].as_u64().unwrap(),
        ],
        stable_backend_node_ids
    );
    assert_eq!(cleared_editor["childNodeCount"], json!(0));
    assert_eq!(
        cleared_clear["attributes"],
        json!([
            "pseudo",
            "-webkit-search-cancel-button",
            "id",
            "search-clear",
            "style",
            "opacity: 0; pointer-events: none;"
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_progress_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        611,
        "<!doctype html><html><body>\
         <progress id='empty'></progress>\
         <progress id='third' value='1' max='3'></progress>\
         <progress id='clamped' value='8' max='4'></progress>\
         <progress id='invalid' value='invalid' max='4'></progress>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 612,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 612);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let empty =
        node_array_element_by_attribute(&body["children"], "id", "empty").expect("empty progress");
    let third =
        node_array_element_by_attribute(&body["children"], "id", "third").expect("third progress");
    let clamped = node_array_element_by_attribute(&body["children"], "id", "clamped")
        .expect("clamped progress");
    let invalid = node_array_element_by_attribute(&body["children"], "id", "invalid")
        .expect("invalid progress");

    let root = &third["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(1));
    let inner = &root["children"][0];
    assert_eq!(
        inner["attributes"],
        json!(["pseudo", "-webkit-progress-inner-element"])
    );
    let bar = &inner["children"][0];
    assert_eq!(bar["attributes"], json!(["pseudo", "-webkit-progress-bar"]));
    let value = &bar["children"][0];
    assert_eq!(
        value["attributes"],
        json!([
            "pseudo",
            "-webkit-progress-value",
            "style",
            "inline-size: 33.3333%; block-size: 100%;"
        ])
    );
    assert_eq!(value["childNodeCount"], json!(0));
    assert_eq!(
        empty["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-progress-value",
            "style",
            "inline-size: -100%; block-size: 100%;"
        ])
    );
    assert_eq!(
        clamped["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-progress-value",
            "style",
            "inline-size: 100%; block-size: 100%;"
        ])
    );
    assert_eq!(
        invalid["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-progress-value",
            "style",
            "inline-size: 0%; block-size: 100%;"
        ])
    );

    let stable_backend_node_ids = [
        root["backendNodeId"].as_u64().expect("root backend id"),
        inner["backendNodeId"].as_u64().expect("inner backend id"),
        bar["backendNodeId"].as_u64().expect("bar backend id"),
        value["backendNodeId"].as_u64().expect("value backend id"),
    ];
    ctx.process_async(json!({
        "id": 613,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "\
                document.getElementById('empty').value=.25;\
                document.getElementById('third').max=6;"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 613);
    ctx.process_async(json!({
        "id": 614,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 614);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_empty = node_array_element_by_attribute(&body["children"], "id", "empty")
        .expect("updated empty progress");
    let updated_third = node_array_element_by_attribute(&body["children"], "id", "third")
        .expect("updated third progress");
    let updated_root = &updated_third["shadowRoots"][0];
    let updated_inner = &updated_root["children"][0];
    let updated_bar = &updated_inner["children"][0];
    let updated_value = &updated_bar["children"][0];
    assert_eq!(
        [
            updated_root["backendNodeId"].as_u64().unwrap(),
            updated_inner["backendNodeId"].as_u64().unwrap(),
            updated_bar["backendNodeId"].as_u64().unwrap(),
            updated_value["backendNodeId"].as_u64().unwrap(),
        ],
        stable_backend_node_ids,
        "Chromium retains the progress UA tree across value and max changes"
    );
    assert_eq!(
        updated_value["attributes"],
        json!([
            "pseudo",
            "-webkit-progress-value",
            "style",
            "inline-size: 16.6667%; block-size: 100%;"
        ])
    );
    assert_eq!(
        updated_empty["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-progress-value",
            "style",
            "inline-size: 25%; block-size: 100%;"
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_meter_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        615,
        "<!doctype html><html><body>\
         <meter id='default'></meter>\
         <meter id='third' value='1' max='3'></meter>\
         <meter id='middle-low' min='0' max='100' low='25' high='75' optimum='50' value='10'></meter>\
         <meter id='middle' min='0' max='100' low='25' high='75' optimum='50' value='50'></meter>\
         <meter id='low-optimum' min='0' max='100' low='25' high='75' optimum='10' value='90'></meter>\
         <meter id='high-optimum' min='0' max='100' low='25' high='75' optimum='90' value='10'></meter>\
         <meter id='offset' min='10' max='20' low='12' high='18' optimum='15' value='15'></meter>\
         <meter id='collapsed' min='10' max='5' value='7'></meter>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 616,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 616);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let default =
        node_array_element_by_attribute(&body["children"], "id", "default").expect("default meter");
    let third =
        node_array_element_by_attribute(&body["children"], "id", "third").expect("third meter");
    let middle_low = node_array_element_by_attribute(&body["children"], "id", "middle-low")
        .expect("middle-range low meter");
    let middle =
        node_array_element_by_attribute(&body["children"], "id", "middle").expect("middle meter");
    let low_optimum = node_array_element_by_attribute(&body["children"], "id", "low-optimum")
        .expect("low-optimum meter");
    let high_optimum = node_array_element_by_attribute(&body["children"], "id", "high-optimum")
        .expect("high-optimum meter");
    let offset =
        node_array_element_by_attribute(&body["children"], "id", "offset").expect("offset meter");
    let collapsed = node_array_element_by_attribute(&body["children"], "id", "collapsed")
        .expect("collapsed meter");

    let root = &third["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(1));
    let inner = &root["children"][0];
    assert_eq!(
        inner["attributes"],
        json!(["pseudo", "-webkit-meter-inner-element"])
    );
    let bar = &inner["children"][0];
    assert_eq!(bar["attributes"], json!(["pseudo", "-webkit-meter-bar"]));
    let value = &bar["children"][0];
    assert_eq!(
        value["attributes"],
        json!([
            "pseudo",
            "-webkit-meter-optimum-value",
            "style",
            "inline-size: 33.3333%; block-size: 100%;"
        ])
    );
    assert_eq!(value["childNodeCount"], json!(0));

    assert_eq!(
        default["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-meter-optimum-value",
            "style",
            "inline-size: 0%; block-size: 100%;"
        ])
    );
    assert_eq!(
        middle_low["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-meter-suboptimum-value",
            "style",
            "inline-size: 10%; block-size: 100%;"
        ])
    );
    assert_eq!(
        low_optimum["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-meter-even-less-good-value",
            "style",
            "inline-size: 90%; block-size: 100%;"
        ])
    );
    assert_eq!(
        high_optimum["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-meter-even-less-good-value",
            "style",
            "inline-size: 10%; block-size: 100%;"
        ])
    );
    assert_eq!(
        offset["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-meter-optimum-value",
            "style",
            "inline-size: 50%; block-size: 100%;"
        ])
    );
    assert_eq!(
        collapsed["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
        json!([
            "pseudo",
            "-webkit-meter-optimum-value",
            "style",
            "inline-size: 0%; block-size: 100%;"
        ])
    );

    let middle_root = &middle["shadowRoots"][0];
    let middle_inner = &middle_root["children"][0];
    let middle_bar = &middle_inner["children"][0];
    let middle_value = &middle_bar["children"][0];
    let stable_backend_node_ids = [
        middle_root["backendNodeId"]
            .as_u64()
            .expect("root backend id"),
        middle_inner["backendNodeId"]
            .as_u64()
            .expect("inner backend id"),
        middle_bar["backendNodeId"]
            .as_u64()
            .expect("bar backend id"),
        middle_value["backendNodeId"]
            .as_u64()
            .expect("value backend id"),
    ];
    ctx.process_async(json!({
        "id": 617,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "\
                const middle = document.getElementById('middle');\
                middle.optimum=10;\
                middle.value=90;"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 617);
    ctx.process_async(json!({
        "id": 618,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 618);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_middle = node_array_element_by_attribute(&body["children"], "id", "middle")
        .expect("updated middle meter");
    let updated_root = &updated_middle["shadowRoots"][0];
    let updated_inner = &updated_root["children"][0];
    let updated_bar = &updated_inner["children"][0];
    let updated_value = &updated_bar["children"][0];
    assert_eq!(
        [
            updated_root["backendNodeId"].as_u64().unwrap(),
            updated_inner["backendNodeId"].as_u64().unwrap(),
            updated_bar["backendNodeId"].as_u64().unwrap(),
            updated_value["backendNodeId"].as_u64().unwrap(),
        ],
        stable_backend_node_ids,
        "Chromium retains the meter UA tree across value and region changes"
    );
    assert_eq!(
        updated_value["attributes"],
        json!([
            "pseudo",
            "-webkit-meter-even-less-good-value",
            "style",
            "inline-size: 90%; block-size: 100%;"
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_optgroup_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        621,
        "<!doctype html><html><body><select>\
         <optgroup id='missing'><option>One</option></optgroup>\
         <optgroup id='empty' label=''><option>Two</option></optgroup>\
         <optgroup id='named' label='Recommended'><option>Three</option></optgroup>\
         <optgroup id='disabled' label='Disabled' disabled><option>Four</option></optgroup>\
         </select></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 622,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 622);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let select = child_element_by_node_name(body, "SELECT");
    let missing = node_array_element_by_attribute(&select["children"], "id", "missing")
        .expect("missing-label optgroup");
    let empty = node_array_element_by_attribute(&select["children"], "id", "empty")
        .expect("empty-label optgroup");
    let named = node_array_element_by_attribute(&select["children"], "id", "named")
        .expect("named optgroup");
    let disabled = node_array_element_by_attribute(&select["children"], "id", "disabled")
        .expect("disabled optgroup");

    let root = &named["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(2));
    let label = &root["children"][0];
    assert_eq!(label["nodeName"], json!("DIV"));
    assert_eq!(
        label["attributes"],
        json!([
            "aria-hidden",
            "true",
            "pseudo",
            "-internal-optgroup-label",
            "aria-label",
            "Recommended"
        ])
    );
    assert_eq!(label["children"][0]["nodeValue"], json!("Recommended"));
    let slot = &root["children"][1];
    assert_eq!(slot["nodeName"], json!("SLOT"));
    assert_eq!(slot["attributes"], json!([]));
    assert_eq!(slot["childNodeCount"], json!(0));

    assert_eq!(
        missing["shadowRoots"][0]["children"][0]["attributes"],
        json!(["aria-hidden", "true", "pseudo", "-internal-optgroup-label"])
    );
    assert_eq!(
        missing["shadowRoots"][0]["children"][0]["childNodeCount"],
        json!(0)
    );
    assert_eq!(
        empty["shadowRoots"][0]["children"][0]["attributes"],
        json!([
            "aria-hidden",
            "true",
            "pseudo",
            "-internal-optgroup-label",
            "aria-label",
            ""
        ])
    );
    assert_eq!(
        empty["shadowRoots"][0]["children"][0]["childNodeCount"],
        json!(0)
    );
    assert_eq!(
        disabled["shadowRoots"][0]["children"][0]["children"][0]["nodeValue"],
        json!("Disabled"),
        "disabled state does not change the optgroup UA tree"
    );

    let stable_backend_node_ids = [
        root["backendNodeId"].as_u64().expect("root backend id"),
        label["backendNodeId"].as_u64().expect("label backend id"),
        slot["backendNodeId"].as_u64().expect("slot backend id"),
    ];
    let label_text_backend_node_id = label["children"][0]["backendNodeId"]
        .as_u64()
        .expect("label text backend id");
    ctx.process_async(json!({
        "id": 623,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "\
                document.getElementById('missing').label='Added';\
                document.getElementById('named').label='Changed';\
                document.getElementById('named').append(document.createElement('option'));"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 623);
    ctx.process_async(json!({
        "id": 624,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 624);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let select = child_element_by_node_name(body, "SELECT");
    let updated_missing = node_array_element_by_attribute(&select["children"], "id", "missing")
        .expect("updated missing-label optgroup");
    let updated_named = node_array_element_by_attribute(&select["children"], "id", "named")
        .expect("updated named optgroup");
    let updated_root = &updated_named["shadowRoots"][0];
    let updated_label = &updated_root["children"][0];
    let updated_slot = &updated_root["children"][1];
    assert_eq!(
        [
            updated_root["backendNodeId"].as_u64().unwrap(),
            updated_label["backendNodeId"].as_u64().unwrap(),
            updated_slot["backendNodeId"].as_u64().unwrap(),
        ],
        stable_backend_node_ids,
        "Chromium retains the optgroup UA structure across label and option changes"
    );
    assert_eq!(
        updated_label["attributes"],
        json!([
            "aria-hidden",
            "true",
            "pseudo",
            "-internal-optgroup-label",
            "aria-label",
            "Changed"
        ])
    );
    assert_eq!(updated_label["children"][0]["nodeValue"], json!("Changed"));
    assert_ne!(
        updated_label["children"][0]["backendNodeId"],
        json!(label_text_backend_node_id),
        "Chromium rebuilds the optgroup label text when label changes"
    );
    assert_eq!(
        updated_missing["shadowRoots"][0]["children"][0]["attributes"],
        json!([
            "aria-hidden",
            "true",
            "pseudo",
            "-internal-optgroup-label",
            "aria-label",
            "Added"
        ])
    );
    assert_eq!(
        updated_missing["shadowRoots"][0]["children"][0]["children"][0]["nodeValue"],
        json!("Added")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_datalist_input_user_agent_decorations() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        631,
        "<!doctype html><html><body>\
         <datalist id='choices'><option value='Alpha'></option></datalist>\
         <div id='not-list'></div>\
         <input id='plain' value='Plain'>\
         <input id='missing' list='missing-list' value='Missing'>\
         <input id='wrong' list='not-list' value='Wrong'>\
         <input id='valid' list='choices' value='Alpha'>\
         <input id='search' type='search' list='choices' value='Find'>\
         <input id='number' type='number' list='choices' value='12'>\
         <input id='password' type='password' list='choices' value='Secret'>\
         <datalist id='dynamic-choices'><option value='Before'></option></datalist>\
         <input id='reassociated' list='dynamic-choices' value='Before'>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 632,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => {\
                const dirty = document.createElement('input');\
                dirty.id = 'dirty';\
                dirty.setAttribute('list', 'choices');\
                dirty.value = 'Dirty';\
                document.body.append(dirty);\
                const dynamic = document.getElementById('dynamic-choices');\
                dynamic.id = 'retired-choices';\
                const replacement = document.createElement('datalist');\
                replacement.id = 'dynamic-choices';\
                replacement.append(new Option('After', 'After'));\
                dynamic.after(replacement);\
                document.getElementById('reassociated').value = 'After';\
                return {\
                    valid: document.getElementById('valid').list?.id ?? null,\
                    number: document.getElementById('number').list?.id ?? null,\
                    password: document.getElementById('password').list?.id ?? null\
                };\
            })()",
            "returnByValue": true
        }
    }))
    .await;
    let associations = take_response_by_id(&mut ctx, 632);
    assert_eq!(
        associations["result"]["result"]["value"],
        json!({
            "valid": "choices",
            "number": "choices",
            "password": null
        }),
        "the list IDREF does not apply to password inputs"
    );

    ctx.process_async(json!({
        "id": 633,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 633);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let plain =
        node_array_element_by_attribute(&body["children"], "id", "plain").expect("plain input");
    let missing = node_array_element_by_attribute(&body["children"], "id", "missing")
        .expect("missing-list input");
    let wrong = node_array_element_by_attribute(&body["children"], "id", "wrong")
        .expect("wrong-target input");
    let valid =
        node_array_element_by_attribute(&body["children"], "id", "valid").expect("valid input");
    let dirty =
        node_array_element_by_attribute(&body["children"], "id", "dirty").expect("dirty input");
    let search =
        node_array_element_by_attribute(&body["children"], "id", "search").expect("search input");
    let number =
        node_array_element_by_attribute(&body["children"], "id", "number").expect("number input");
    let password = node_array_element_by_attribute(&body["children"], "id", "password")
        .expect("password input");
    let reassociated = node_array_element_by_attribute(&body["children"], "id", "reassociated")
        .expect("reassociated input");

    for (input, value) in [
        (plain, "Plain"),
        (missing, "Missing"),
        (wrong, "Wrong"),
        (password, "Secret"),
    ] {
        let editor = &input["shadowRoots"][0]["children"][0];
        assert_eq!(editor["attributes"], json!([]));
        assert_eq!(editor["children"][0]["nodeValue"], json!(value));
    }

    let root = &valid["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(1));
    let container = &root["children"][0];
    assert_eq!(
        container["attributes"],
        json!([
            "id",
            "text-field-container",
            "pseudo",
            "-webkit-textfield-decoration-container",
            "style",
            "unicode-bidi: normal;"
        ])
    );
    assert_eq!(container["childNodeCount"], json!(2));
    let viewport = &container["children"][0];
    assert_eq!(viewport["attributes"], json!(["id", "editing-view-port"]));
    let editor = &viewport["children"][0];
    assert_eq!(editor["attributes"], json!([]));
    assert_eq!(editor["children"][0]["nodeValue"], json!("Alpha"));
    let picker = &container["children"][1];
    assert_eq!(
        picker["attributes"],
        json!([
            "pseudo",
            "-webkit-calendar-picker-indicator",
            "id",
            "picker",
            "aria-hidden",
            "true",
            "style",
            "display: list-item; list-style: inside disclosure-open; counter-increment: list-item 0; block-size: 1em;"
        ])
    );
    assert_eq!(picker["childNodeCount"], json!(0));
    assert_eq!(picker["pseudoElements"][0]["nodeName"], json!("::marker"));
    assert_eq!(picker["pseudoElements"][0]["pseudoType"], json!("marker"));
    assert_eq!(
        dirty["shadowRoots"][0]["children"][0]["attributes"],
        json!([
            "id",
            "text-field-container",
            "pseudo",
            "-webkit-textfield-decoration-container"
        ]),
        "Chromium omits the initial unicode-bidi projection after a pre-connection value setter"
    );
    assert_eq!(
        reassociated["shadowRoots"][0]["children"][0]["attributes"], container["attributes"],
        "Chromium initializes a replacement datalist decoration before the next value setter"
    );

    let search_container = &search["shadowRoots"][0]["children"][0];
    assert_eq!(search_container["childNodeCount"], json!(3));
    assert_eq!(
        search_container["children"][1]["attributes"],
        json!([
            "pseudo",
            "-webkit-search-cancel-button",
            "id",
            "search-clear"
        ])
    );
    assert_eq!(
        search_container["children"][2]["attributes"],
        picker["attributes"]
    );
    let number_container = &number["shadowRoots"][0]["children"][0];
    assert_eq!(number_container["childNodeCount"], json!(3));
    assert_eq!(
        number_container["children"][1]["attributes"],
        picker["attributes"]
    );
    assert_eq!(
        number_container["children"][2]["attributes"],
        json!(["pseudo", "-webkit-inner-spin-button", "id", "spin"])
    );

    let stable_backend_node_ids = [
        root["backendNodeId"].as_u64().expect("root backend id"),
        container["backendNodeId"]
            .as_u64()
            .expect("container backend id"),
        viewport["backendNodeId"]
            .as_u64()
            .expect("viewport backend id"),
        editor["backendNodeId"].as_u64().expect("editor backend id"),
        picker["backendNodeId"].as_u64().expect("picker backend id"),
        picker["pseudoElements"][0]["backendNodeId"]
            .as_u64()
            .expect("picker marker backend id"),
    ];
    let text_backend_node_id = editor["children"][0]["backendNodeId"]
        .as_u64()
        .expect("editor text backend id");
    ctx.process_async(json!({
        "id": 634,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('valid').value='Changed'" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 634);
    ctx.process_async(json!({
        "id": 635,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 635);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_valid = node_array_element_by_attribute(&body["children"], "id", "valid")
        .expect("updated valid input");
    let updated_root = &updated_valid["shadowRoots"][0];
    let updated_container = &updated_root["children"][0];
    let updated_viewport = &updated_container["children"][0];
    let updated_editor = &updated_viewport["children"][0];
    let updated_picker = &updated_container["children"][1];
    assert_eq!(
        updated_container["attributes"], container["attributes"],
        "Chromium retains the datalist decoration's initial unicode-bidi projection"
    );
    assert_eq!(
        [
            updated_root["backendNodeId"].as_u64().unwrap(),
            updated_container["backendNodeId"].as_u64().unwrap(),
            updated_viewport["backendNodeId"].as_u64().unwrap(),
            updated_editor["backendNodeId"].as_u64().unwrap(),
            updated_picker["backendNodeId"].as_u64().unwrap(),
            updated_picker["pseudoElements"][0]["backendNodeId"]
                .as_u64()
                .unwrap(),
        ],
        stable_backend_node_ids,
        "Chromium retains the datalist input decoration while value changes"
    );
    assert_eq!(updated_editor["children"][0]["nodeValue"], json!("Changed"));
    assert_ne!(
        updated_editor["children"][0]["backendNodeId"],
        json!(text_backend_node_id),
        "Chromium rebuilds the editor text node while the decoration stays stable"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_range_input_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        641,
        "<!doctype html><html><body>\
         <datalist id='ticks'><option value='25'></option></datalist>\
         <input id='default' type='range'>\
         <input id='bounded' type='range' min='10' max='90' step='5' value='40'>\
         <input id='listed' type='range' list='ticks' value='25'>\
         <input id='disabled' type='range' disabled value='75'>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 642,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 642);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let default =
        node_array_element_by_attribute(&body["children"], "id", "default").expect("default range");
    let bounded =
        node_array_element_by_attribute(&body["children"], "id", "bounded").expect("bounded range");
    let listed =
        node_array_element_by_attribute(&body["children"], "id", "listed").expect("listed range");
    let disabled = node_array_element_by_attribute(&body["children"], "id", "disabled")
        .expect("disabled range");

    let root = &bounded["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(1));
    let container = &root["children"][0];
    assert_eq!(container["nodeName"], json!("DIV"));
    assert_eq!(container["attributes"], json!([]));
    assert_eq!(container["childNodeCount"], json!(1));
    let track = &container["children"][0];
    assert_eq!(
        track["attributes"],
        json!(["pseudo", "-webkit-slider-runnable-track", "id", "track"])
    );
    let thumb = &track["children"][0];
    assert_eq!(thumb["attributes"], json!(["id", "thumb"]));
    assert_eq!(thumb["childNodeCount"], json!(0));

    for range in [default, listed, disabled] {
        assert_eq!(
            range["shadowRoots"][0]["children"][0]["children"][0]["attributes"],
            track["attributes"]
        );
        assert_eq!(
            range["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["attributes"],
            thumb["attributes"]
        );
    }

    let stable_backend_node_ids = [
        root["backendNodeId"].as_u64().expect("root backend id"),
        container["backendNodeId"]
            .as_u64()
            .expect("container backend id"),
        track["backendNodeId"].as_u64().expect("track backend id"),
        thumb["backendNodeId"].as_u64().expect("thumb backend id"),
    ];
    ctx.process_async(json!({
        "id": 643,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "\
                const bounded = document.getElementById('bounded');\
                bounded.value='80';\
                bounded.min='0';\
                bounded.max='200';\
                bounded.step='10';\
                document.getElementById('listed').removeAttribute('list');"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 643);
    ctx.process_async(json!({
        "id": 644,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 644);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_bounded = node_array_element_by_attribute(&body["children"], "id", "bounded")
        .expect("updated bounded range");
    let updated_root = &updated_bounded["shadowRoots"][0];
    let updated_container = &updated_root["children"][0];
    let updated_track = &updated_container["children"][0];
    let updated_thumb = &updated_track["children"][0];
    assert_eq!(
        [
            updated_root["backendNodeId"].as_u64().unwrap(),
            updated_container["backendNodeId"].as_u64().unwrap(),
            updated_track["backendNodeId"].as_u64().unwrap(),
            updated_thumb["backendNodeId"].as_u64().unwrap(),
        ],
        stable_backend_node_ids,
        "Chromium retains the range UA tree across value and constraint changes"
    );
    assert_eq!(updated_track["attributes"], track["attributes"]);
    assert_eq!(updated_thumb["attributes"], thumb["attributes"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_number_input_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        61,
        "<!doctype html><html><body>\
         <input id='number' type='number' value='4'>\
         <input id='empty' type='number'>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 62,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 62);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let number =
        node_array_element_by_attribute(&body["children"], "id", "number").expect("number input");
    let empty =
        node_array_element_by_attribute(&body["children"], "id", "empty").expect("empty input");

    let root = &number["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(1));
    let container = &root["children"][0];
    assert_eq!(container["nodeName"], json!("DIV"));
    assert_eq!(
        container["attributes"],
        json!([
            "id",
            "text-field-container",
            "pseudo",
            "-webkit-textfield-decoration-container",
            "style",
            "unicode-bidi: normal;"
        ])
    );
    assert_eq!(container["childNodeCount"], json!(2));
    let viewport = &container["children"][0];
    assert_eq!(viewport["attributes"], json!(["id", "editing-view-port"]));
    let editor = &viewport["children"][0];
    assert_eq!(editor["attributes"], json!([]));
    assert_eq!(editor["children"][0]["nodeValue"], json!("4"));
    let spin = &container["children"][1];
    assert_eq!(
        spin["attributes"],
        json!(["pseudo", "-webkit-inner-spin-button", "id", "spin"])
    );
    assert_eq!(
        empty["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["childNodeCount"],
        json!(0)
    );

    let stable_backend_node_ids = [
        root["backendNodeId"].as_u64().expect("root backend id"),
        container["backendNodeId"]
            .as_u64()
            .expect("container backend id"),
        viewport["backendNodeId"]
            .as_u64()
            .expect("viewport backend id"),
        editor["backendNodeId"].as_u64().expect("editor backend id"),
        spin["backendNodeId"].as_u64().expect("spin backend id"),
    ];
    let text_backend_node_id = editor["children"][0]["backendNodeId"]
        .as_u64()
        .expect("text backend id");
    ctx.process_async(json!({
        "id": 63,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('number').value='17'; document.getElementById('empty').value='9'"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 63);
    ctx.process_async(json!({
        "id": 64,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 64);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_number =
        node_array_element_by_attribute(&body["children"], "id", "number").expect("updated number");
    let updated_empty =
        node_array_element_by_attribute(&body["children"], "id", "empty").expect("updated empty");
    let updated_root = &updated_number["shadowRoots"][0];
    let updated_container = &updated_root["children"][0];
    let updated_viewport = &updated_container["children"][0];
    let updated_editor = &updated_viewport["children"][0];
    let updated_spin = &updated_container["children"][1];
    assert_eq!(
        [
            updated_root["backendNodeId"].as_u64().unwrap(),
            updated_container["backendNodeId"].as_u64().unwrap(),
            updated_viewport["backendNodeId"].as_u64().unwrap(),
            updated_editor["backendNodeId"].as_u64().unwrap(),
            updated_spin["backendNodeId"].as_u64().unwrap(),
        ],
        stable_backend_node_ids
    );
    assert_eq!(updated_editor["children"][0]["nodeValue"], json!("17"));
    assert_ne!(
        updated_editor["children"][0]["backendNodeId"],
        json!(text_backend_node_id),
        "Chromium rebuilds the number editor text node when value changes"
    );
    assert_eq!(
        updated_empty["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["children"][0]
            ["nodeValue"],
        json!("9")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_date_input_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        65,
        "<!doctype html><html><body>\
         <input id='date' type='date' value='2026-08-12'>\
         <input id='empty' type='date'>\
         <input id='bounded' type='date' value='2026-08-12' min='2020-02-03' max='2030-10-20'>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 66,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 66);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let date =
        node_array_element_by_attribute(&body["children"], "id", "date").expect("date input");
    let empty =
        node_array_element_by_attribute(&body["children"], "id", "empty").expect("empty date");
    let bounded =
        node_array_element_by_attribute(&body["children"], "id", "bounded").expect("bounded date");

    let root = &date["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(1));
    let container = &root["children"][0];
    assert_eq!(
        container["attributes"],
        json!([
            "pseudo",
            "-internal-datetime-container",
            "style",
            "unicode-bidi: normal;"
        ])
    );
    assert_eq!(container["childNodeCount"], json!(2));
    let edit = &container["children"][0];
    assert_eq!(
        edit["attributes"],
        json!([
            "pseudo",
            "-webkit-datetime-edit",
            "id",
            "date-time-edit",
            "datetimeformat",
            "M/d/yy",
            "style",
            "unicode-bidi: normal;"
        ])
    );
    let fields = &edit["children"][0];
    assert_eq!(
        fields["attributes"],
        json!([
            "pseudo",
            "-webkit-datetime-edit-fields-wrapper",
            "style",
            "unicode-bidi: normal;"
        ])
    );
    assert_eq!(fields["childNodeCount"], json!(5));
    assert_eq!(
        fields["children"][0]["attributes"],
        json!([
            "role",
            "spinbutton",
            "aria-placeholder",
            "mm",
            "aria-valuemin",
            "1",
            "aria-valuemax",
            "12",
            "aria-label",
            "Month",
            "pseudo",
            "-webkit-datetime-edit-month-field",
            "aria-valuenow",
            "8",
            "aria-valuetext",
            "08"
        ])
    );
    assert_eq!(
        fields["children"][0]["children"][0]["nodeValue"],
        json!("08")
    );
    assert_eq!(
        fields["children"][1]["attributes"],
        json!([
            "pseudo",
            "-webkit-datetime-edit-text",
            "style",
            "unicode-bidi: normal;"
        ])
    );
    assert_eq!(
        fields["children"][1]["children"][0]["nodeValue"],
        json!("/")
    );
    assert_eq!(
        fields["children"][2]["attributes"],
        json!([
            "role",
            "spinbutton",
            "aria-placeholder",
            "dd",
            "aria-valuemin",
            "1",
            "aria-valuemax",
            "31",
            "aria-label",
            "Day",
            "pseudo",
            "-webkit-datetime-edit-day-field",
            "aria-valuenow",
            "12",
            "aria-valuetext",
            "12"
        ])
    );
    assert_eq!(
        fields["children"][2]["children"][0]["nodeValue"],
        json!("12")
    );
    assert_eq!(
        fields["children"][4]["attributes"],
        json!([
            "role",
            "spinbutton",
            "aria-placeholder",
            "yyyy",
            "aria-valuemin",
            "1",
            "aria-valuemax",
            "275760",
            "aria-label",
            "Year",
            "pseudo",
            "-webkit-datetime-edit-year-field",
            "aria-valuenow",
            "2026",
            "aria-valuetext",
            "2026"
        ])
    );
    assert_eq!(
        fields["children"][4]["children"][0]["nodeValue"],
        json!("2026")
    );
    let picker = &container["children"][1];
    assert_eq!(
        picker["attributes"],
        json!([
            "pseudo",
            "-webkit-calendar-picker-indicator",
            "id",
            "picker",
            "tabindex",
            "0",
            "aria-haspopup",
            "menu",
            "role",
            "button",
            "title",
            "Show date picker"
        ])
    );

    let empty_fields = &empty["shadowRoots"][0]["children"][0]["children"][0]["children"][0];
    assert_eq!(
        empty_fields["children"][0]["attributes"],
        json!([
            "role",
            "spinbutton",
            "aria-placeholder",
            "mm",
            "aria-valuemin",
            "1",
            "aria-valuemax",
            "12",
            "aria-label",
            "Month",
            "pseudo",
            "-webkit-datetime-edit-month-field"
        ])
    );
    assert_eq!(
        empty_fields["children"][0]["children"][0]["nodeValue"],
        json!("mm")
    );
    assert_eq!(
        empty_fields["children"][2]["children"][0]["nodeValue"],
        json!("dd")
    );
    assert_eq!(
        empty_fields["children"][4]["children"][0]["nodeValue"],
        json!("yyyy")
    );
    let bounded_year =
        &bounded["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["children"][4];
    assert_eq!(bounded_year["attributes"][5], json!("2020"));
    assert_eq!(bounded_year["attributes"][7], json!("2030"));

    let stable_backend_node_ids = [
        root["backendNodeId"].as_u64().expect("root backend id"),
        container["backendNodeId"]
            .as_u64()
            .expect("container backend id"),
        edit["backendNodeId"].as_u64().expect("edit backend id"),
        fields["backendNodeId"].as_u64().expect("fields backend id"),
        picker["backendNodeId"].as_u64().expect("picker backend id"),
    ];
    let dynamic_backend_node_ids = fields["children"]
        .as_array()
        .expect("date fields")
        .iter()
        .map(|node| node["backendNodeId"].as_u64().expect("dynamic backend id"))
        .collect::<Vec<_>>();
    ctx.process_async(json!({
        "id": 67,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('date').value='2031-01-02'; document.getElementById('empty').value='2024-03-04'; document.getElementById('bounded').min='2021-01-01'; document.getElementById('bounded').max='2029-12-31'"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 67);
    ctx.process_async(json!({
        "id": 68,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let updated = take_response_by_id(&mut ctx, 68);
    let html = child_element_by_node_name(&updated["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let updated_date =
        node_array_element_by_attribute(&body["children"], "id", "date").expect("updated date");
    let updated_empty =
        node_array_element_by_attribute(&body["children"], "id", "empty").expect("updated empty");
    let updated_bounded = node_array_element_by_attribute(&body["children"], "id", "bounded")
        .expect("updated bounded");
    let updated_root = &updated_date["shadowRoots"][0];
    let updated_container = &updated_root["children"][0];
    let updated_edit = &updated_container["children"][0];
    let updated_fields = &updated_edit["children"][0];
    let updated_picker = &updated_container["children"][1];
    assert_eq!(
        [
            updated_root["backendNodeId"].as_u64().unwrap(),
            updated_container["backendNodeId"].as_u64().unwrap(),
            updated_edit["backendNodeId"].as_u64().unwrap(),
            updated_fields["backendNodeId"].as_u64().unwrap(),
            updated_picker["backendNodeId"].as_u64().unwrap(),
        ],
        stable_backend_node_ids
    );
    let updated_dynamic_backend_node_ids = updated_fields["children"]
        .as_array()
        .expect("updated date fields")
        .iter()
        .map(|node| node["backendNodeId"].as_u64().expect("dynamic backend id"))
        .collect::<Vec<_>>();
    assert!(
        dynamic_backend_node_ids
            .iter()
            .zip(&updated_dynamic_backend_node_ids)
            .all(|(before, after)| before != after),
        "Chromium rebuilds all date field and separator elements when value changes"
    );
    assert_eq!(
        updated_fields["children"][0]["children"][0]["nodeValue"],
        json!("01")
    );
    assert_eq!(
        updated_fields["children"][2]["children"][0]["nodeValue"],
        json!("02")
    );
    assert_eq!(
        updated_fields["children"][4]["children"][0]["nodeValue"],
        json!("2031")
    );
    assert_eq!(
        updated_empty["shadowRoots"][0]["children"][0]["children"][0]["children"][0]["children"][0]
            ["children"][0]["nodeValue"],
        json!("03")
    );
    let updated_bounded_year = &updated_bounded["shadowRoots"][0]["children"][0]["children"][0]["children"]
        [0]["children"][4];
    assert_eq!(updated_bounded_year["attributes"][5], json!("2021"));
    assert_eq!(updated_bounded_year["attributes"][7], json!("2029"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_projects_details_user_agent_shadow_roots() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        69,
        "<!doctype html><html><body>\
         <details id='closed'><summary id='author'>Author</summary><p>Body</p></details>\
         <details id='open' open><summary>Open author</summary><p>Body</p></details>\
         <details id='fallback'><p>No author summary</p></details>\
         <details id='multi'><summary>First</summary><summary>Second</summary></details>\
         </body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 70,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 70);
    let html = child_element_by_node_name(&response["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let closed =
        node_array_element_by_attribute(&body["children"], "id", "closed").expect("closed details");
    let open =
        node_array_element_by_attribute(&body["children"], "id", "open").expect("open details");
    let fallback = node_array_element_by_attribute(&body["children"], "id", "fallback")
        .expect("fallback details");
    let multi =
        node_array_element_by_attribute(&body["children"], "id", "multi").expect("multi details");

    let root = &closed["shadowRoots"][0];
    assert_eq!(root["shadowRootType"], json!("user-agent"));
    assert_eq!(root["childNodeCount"], json!(3));
    let summary_slot = &root["children"][0];
    assert_eq!(summary_slot["nodeName"], json!("SLOT"));
    assert_eq!(summary_slot["attributes"], json!(["id", "details-summary"]));
    assert_eq!(summary_slot["childNodeCount"], json!(1));
    let fallback_summary = &summary_slot["children"][0];
    assert_eq!(fallback_summary["nodeName"], json!("SUMMARY"));
    assert_eq!(fallback_summary["attributes"], json!([]));
    assert_eq!(
        fallback_summary["children"][0]["nodeValue"],
        json!("Details")
    );
    assert!(
        fallback_summary.get("pseudoElements").is_none(),
        "an author summary suppresses the fallback summary marker"
    );

    let content_slot = &root["children"][1];
    assert_eq!(content_slot["nodeName"], json!("SLOT"));
    assert_eq!(
        content_slot["attributes"],
        json!([
            "id",
            "details-content",
            "pseudo",
            "details-content",
            "style",
            "content-visibility: hidden; display: block;"
        ])
    );
    assert_eq!(
        open["shadowRoots"][0]["children"][1]["attributes"],
        json!([
            "id",
            "details-content",
            "pseudo",
            "details-content",
            "style",
            "display: block;"
        ])
    );

    let style = &root["children"][2];
    assert_eq!(style["nodeName"], json!("STYLE"));
    assert_eq!(style["attributes"], json!([]));
    assert_eq!(
        style["children"][0]["nodeValue"],
        json!(
            "\n:host summary {\n  display: list-item;\n  counter-increment: list-item 0;\n  list-style: disclosure-closed inside;\n}\n:host([open]) summary {\n  list-style-type: disclosure-open;\n}\n"
        )
    );
    let fallback_summary = &fallback["shadowRoots"][0]["children"][0]["children"][0];
    let fallback_marker = &fallback_summary["pseudoElements"][0];
    assert_eq!(fallback_marker["nodeName"], json!("::marker"));
    assert_eq!(fallback_marker["localName"], json!("::marker"));
    assert_eq!(fallback_marker["pseudoType"], json!("marker"));
    assert_eq!(fallback_marker["attributes"], json!([]));
    assert!(
        multi["shadowRoots"][0]["children"][0]["children"][0]
            .get("pseudoElements")
            .is_none(),
        "the first of multiple author summaries still suppresses the fallback marker"
    );

    let stable_backend_node_ids = [
        root["backendNodeId"].as_u64().expect("root backend id"),
        summary_slot["backendNodeId"]
            .as_u64()
            .expect("summary slot backend id"),
        summary_slot["children"][0]["backendNodeId"]
            .as_u64()
            .expect("fallback summary backend id"),
        summary_slot["children"][0]["children"][0]["backendNodeId"]
            .as_u64()
            .expect("fallback text backend id"),
        content_slot["backendNodeId"]
            .as_u64()
            .expect("content slot backend id"),
        style["backendNodeId"].as_u64().expect("style backend id"),
        style["children"][0]["backendNodeId"]
            .as_u64()
            .expect("style text backend id"),
    ];
    ctx.process_async(json!({
        "id": 71,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.getElementById('closed').open=true; document.getElementById('open').open=false"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 71);
    ctx.process_async(json!({
        "id": 72,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let toggled = take_response_by_id(&mut ctx, 72);
    let html = child_element_by_node_name(&toggled["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let toggled_closed = node_array_element_by_attribute(&body["children"], "id", "closed")
        .expect("toggled closed details");
    let toggled_open = node_array_element_by_attribute(&body["children"], "id", "open")
        .expect("toggled open details");
    let toggled_root = &toggled_closed["shadowRoots"][0];
    assert_eq!(
        [
            toggled_root["backendNodeId"].as_u64().unwrap(),
            toggled_root["children"][0]["backendNodeId"]
                .as_u64()
                .unwrap(),
            toggled_root["children"][0]["children"][0]["backendNodeId"]
                .as_u64()
                .unwrap(),
            toggled_root["children"][0]["children"][0]["children"][0]["backendNodeId"]
                .as_u64()
                .unwrap(),
            toggled_root["children"][1]["backendNodeId"]
                .as_u64()
                .unwrap(),
            toggled_root["children"][2]["backendNodeId"]
                .as_u64()
                .unwrap(),
            toggled_root["children"][2]["children"][0]["backendNodeId"]
                .as_u64()
                .unwrap(),
        ],
        stable_backend_node_ids,
        "open state changes only the content slot style"
    );
    assert_eq!(
        toggled_root["children"][1]["attributes"][5],
        json!("display: block;")
    );
    let toggled_open_style = toggled_open["shadowRoots"][0]["children"][1]["attributes"][5]
        .as_str()
        .expect("closed content style");
    assert!(
        matches!(
            toggled_open_style,
            "content-visibility: hidden; display: block;"
                | "display: block; content-visibility: hidden;"
        ),
        "CSS declaration order is not part of this field-level contract: {toggled_open_style}"
    );

    ctx.process_async(json!({
        "id": 73,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.getElementById('author').remove()" }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 73);
    ctx.process_async(json!({
        "id": 74,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    }))
    .await;
    let removed = take_response_by_id(&mut ctx, 74);
    let html = child_element_by_node_name(&removed["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let removed_closed = node_array_element_by_attribute(&body["children"], "id", "closed")
        .expect("details without author summary");
    let removed_fallback_summary = &removed_closed["shadowRoots"][0]["children"][0]["children"][0];
    assert_eq!(
        removed_fallback_summary["backendNodeId"],
        json!(stable_backend_node_ids[2]),
        "fallback summary remains the same generated node"
    );
    let marker_backend_node_id = removed_fallback_summary["pseudoElements"][0]["backendNodeId"]
        .as_u64()
        .expect("fallback marker backend id");
    ctx.process_async(json!({
        "id": 75,
        "method": "DOM.describeNode",
        "params": {
            "backendNodeId": stable_backend_node_ids[2],
            "depth": 0,
            "pierce": false
        }
    }))
    .await;
    let described_summary = take_response_by_id(&mut ctx, 75);
    assert_eq!(
        described_summary["result"]["node"]["pseudoElements"][0]["backendNodeId"],
        json!(marker_backend_node_id),
        "depth-zero describe retains the shallow pseudo-element association"
    );
    ctx.process_async(json!({
        "id": 76,
        "method": "DOM.describeNode",
        "params": { "backendNodeId": marker_backend_node_id }
    }))
    .await;
    let described_marker = take_response_by_id(&mut ctx, 76);
    assert_eq!(
        described_marker["result"]["node"]["pseudoType"],
        json!("marker")
    );
    assert_eq!(
        described_marker["result"]["node"]["backendNodeId"],
        json!(marker_backend_node_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_depth_one_matches_registry_writer_shape() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        27,
        "<a id='a1'></a><div id='d2'><a id='a2'></a></div>",
    )
    .await;

    ctx.process_async(json!({
        "id": 28,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 28);
    let root = &response["result"]["root"];
    assert_eq!(root["nodeId"], json!(1));
    let root_backend_node_id = root["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("root backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(root_backend_node_id),
        "DOM.getDocument should assign renderer backend ids to live document payload: {root}"
    );
    assert_eq!(root["nodeName"], json!("#document"));
    assert_eq!(root["childNodeCount"], json!(1));

    ctx.process_async(json!({
        "id": 29,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": 1, "depth": 1 }
    }))
    .await;
    let document_children = ctx.take_first_matching("document child nodes", |message| {
        message["method"] == json!("DOM.setChildNodes") && message["params"]["parentId"] == json!(1)
    });
    assert_eq!(document_children["method"], "DOM.setChildNodes");
    assert_eq!(document_children["params"]["parentId"], json!(1));
    let root_children = document_children["params"]["nodes"]
        .as_array()
        .expect("document children");
    assert_eq!(root_children.len(), 1);
    assert_eq!(root_children[0]["nodeId"], json!(2));
    let html_backend_node_id = root_children[0]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("html backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(html_backend_node_id),
        "DOM.requestChildNodes should assign renderer backend ids to live child payload: {document_children}"
    );
    assert_eq!(root_children[0]["nodeName"], json!("HTML"));
    assert_eq!(root_children[0]["childNodeCount"], json!(2));
    ctx.expect_result(29, json!({}), None);

    ctx.process_async(json!({
        "id": 30,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": 2, "depth": 1 }
    }))
    .await;
    let child_nodes = ctx.take_first_matching("HTML child nodes", |message| {
        message["method"] == json!("DOM.setChildNodes") && message["params"]["parentId"] == json!(2)
    });
    assert_eq!(child_nodes["method"], "DOM.setChildNodes");
    assert_eq!(child_nodes["params"]["parentId"], json!(2));
    let html_children = child_nodes["params"]["nodes"]
        .as_array()
        .expect("html children");
    assert_eq!(html_children.len(), 2);
    assert_eq!(html_children[0]["nodeId"], json!(3));
    assert_eq!(html_children[0]["nodeName"], json!("HEAD"));
    assert_eq!(html_children[1]["nodeId"], json!(4));
    assert_eq!(html_children[1]["nodeName"], json!("BODY"));
    ctx.expect_result(30, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_renderer_backend_ids_resolve_live_nodes() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='target'>live payload</section></body></html>",
    )
    .await;

    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 3);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let target = child_element_by_node_name(body, "SECTION");
    let backend_node_id = target["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("target backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "DOM.getDocument element backend id should be renderer-owned: {target}"
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.resolveNode",
        "params": { "backendNodeId": backend_node_id }
    }))
    .await;
    let resolved = take_response_by_id(&mut ctx, 4);
    let object_id = resolved["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_else(|| panic!("backend id should resolve to runtime object: {resolved}"))
        .to_owned();

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.id + '|' + this.textContent; }",
            "returnByValue": true
        }
    }))
    .await;
    let checked = take_response_by_id(&mut ctx, 5);
    assert_eq!(
        checked["result"]["result"]["value"],
        json!("target|live payload")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_registers_renderer_frontend_bindings_for_node_consumers() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><section id='target'>live payload</section></body></html>",
    )
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 2);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let target = child_element_by_node_name(body, "SECTION");
    let frontend_node_id = target["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("target frontend node id");
    let backend_node_id = target["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("target backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "DOM.getDocument element backend id should be renderer-owned: {target}"
    );

    ctx.conn
        .clear_runtime_remote_object_tracking_for_session_owner(None);

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getAttributes",
        "params": { "nodeId": frontend_node_id }
    }))
    .await;
    ctx.expect_result(3, json!({ "attributes": ["id", "target"] }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn request_child_nodes_registers_renderer_frontend_bindings_for_node_consumers() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><main id='main'><section id='target'></section></main></body></html>",
    )
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 2);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let html_node_id = html["nodeId"].as_u64().expect("html node id");

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": html_node_id, "depth": 2 }
    }))
    .await;
    let set_child_nodes = ctx.take_one();
    assert_eq!(set_child_nodes["method"], json!("DOM.setChildNodes"));
    assert_eq!(
        set_child_nodes["params"]["parentId"],
        json!(html_node_id),
        "requestChildNodes should emit children for the requested HTML node"
    );
    let body = node_array_element_by_node_name(&set_child_nodes["params"]["nodes"], "BODY");
    let main = child_element_by_node_name(body, "MAIN");
    let frontend_node_id = main["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("main frontend node id");
    let backend_node_id = main["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("main backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "DOM.requestChildNodes nested element backend id should be renderer-owned: {main}"
    );
    ctx.expect_result(3, json!({}), None);

    ctx.conn
        .clear_runtime_remote_object_tracking_for_session_owner(None);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.getAttributes",
        "params": { "nodeId": frontend_node_id }
    }))
    .await;
    ctx.expect_result(4, json!({ "attributes": ["id", "main"] }), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn document_query_search_and_frame_owner_paths_use_explicit_dispatch() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><main id='main'><section id='target'><p class='hit'>one</p><p class='hit'>two</p></section></main></body></html>",
    )
    .await;

    let document = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 2,
            "method": "DOM.getDocument",
            "params": { "depth": -1 }
        }),
        "DOM.getDocument without child frame owners should not use legacy fallback",
    )
    .await;
    let root = &document[0]["result"]["root"];
    let html = child_element_by_node_name(root, "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let main = child_element_by_node_name(body, "MAIN");
    let target = child_element_by_node_name(main, "SECTION");
    let mut html_node_id = html["nodeId"].as_u64().expect("html node id");
    let mut body_node_id = body["nodeId"].as_u64().expect("body node id");
    let mut target_node_id = target["nodeId"].as_u64().expect("target node id");
    let target_backend_node_id = target["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("target backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(target_backend_node_id),
        "DOM.getDocument should return renderer backend ids: {document:?}"
    );

    let flattened = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 3,
            "method": "DOM.getFlattenedDocument",
            "params": { "depth": -1 }
        }),
        "DOM.getFlattenedDocument without child frame owners should not use legacy fallback",
    )
    .await;
    assert!(
        node_array_element_by_attribute(&flattened[0]["result"]["nodes"], "id", "target").is_some(),
        "flattened document should include target section"
    );

    let refreshed = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 30,
            "method": "DOM.getDocument",
            "params": { "depth": -1 }
        }),
        "DOM.getDocument should rebuild frontend bindings after flattened document capture",
    )
    .await;
    let refreshed_root = &refreshed[0]["result"]["root"];
    let refreshed_html = child_element_by_node_name(refreshed_root, "HTML");
    let refreshed_body = child_element_by_node_name(refreshed_html, "BODY");
    let refreshed_main = child_element_by_node_name(refreshed_body, "MAIN");
    let refreshed_target = child_element_by_node_name(refreshed_main, "SECTION");
    let old_html_node_id = html_node_id;
    let old_body_node_id = body_node_id;
    let old_target_node_id = target_node_id;
    html_node_id = refreshed_html["nodeId"]
        .as_u64()
        .expect("refreshed html node id");
    body_node_id = refreshed_body["nodeId"]
        .as_u64()
        .expect("refreshed body node id");
    target_node_id = refreshed_target["nodeId"]
        .as_u64()
        .expect("refreshed target node id");
    assert_ne!(html_node_id, old_html_node_id);
    assert_ne!(body_node_id, old_body_node_id);
    assert_ne!(target_node_id, old_target_node_id);

    let query = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 4,
            "method": "DOM.querySelector",
            "params": { "nodeId": body_node_id, "selector": "#target" }
        }),
        "DOM.querySelector without child frame siblings should not use legacy fallback",
    )
    .await;
    assert_eq!(
        query,
        vec![json!({ "id": 4, "result": { "nodeId": target_node_id } })],
        "a fully published document must not emit duplicate child snapshots"
    );

    let query_all = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 5,
            "method": "DOM.querySelectorAll",
            "params": { "nodeId": target_node_id, "selector": "p.hit" }
        }),
        "DOM.querySelectorAll without child frame siblings should not use legacy fallback",
    )
    .await;
    assert_eq!(query_all.len(), 1);
    assert_eq!(query_all[0]["id"], json!(5));
    assert_eq!(
        query_all[0]["result"]["nodeIds"]
            .as_array()
            .expect("node ids")
            .len(),
        2
    );

    let request_child_nodes = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 6,
            "method": "DOM.requestChildNodes",
            "params": { "nodeId": html_node_id, "depth": 1 }
        }),
        "DOM.requestChildNodes without child frame owners should not use legacy fallback",
    )
    .await;
    assert_eq!(request_child_nodes[0]["method"], json!("DOM.setChildNodes"));
    assert_eq!(
        request_child_nodes[0]["params"]["parentId"],
        json!(html_node_id)
    );
    assert_eq!(request_child_nodes[1], json!({ "id": 6, "result": {} }));

    let perform_search = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 7,
            "method": "DOM.performSearch",
            "params": {
                "query": "p.hit",
                "includeUserAgentShadowDOM": true
            }
        }),
        "DOM.performSearch without child frame owners should not use legacy fallback",
    )
    .await;
    assert_eq!(
        perform_search[0]["result"],
        json!({ "searchId": "0", "resultCount": 2 })
    );
    assert!(
        perform_search
            .iter()
            .all(|message| message["method"] != json!("DOM.setChildNodes")),
        "includeUserAgentShadowDOM should not request child-node snapshots"
    );

    ctx.process_async(json!({
        "id": 70,
        "method": "DOM.getSearchResults",
        "params": { "searchId": "0", "fromIndex": 0, "toIndex": 2 }
    }))
    .await;
    let search_results = take_response_by_id(&mut ctx, 70);
    assert_eq!(
        search_results["result"]["nodeIds"].as_array().map(Vec::len),
        Some(2)
    );

    let frame_owner = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 8,
            "method": "DOM.getFrameOwner",
            "params": { "frameId": "TID-1" }
        }),
        "DOM.getFrameOwner top frame should not use legacy fallback",
    )
    .await;
    assert_eq!(
        frame_owner[0]["error"],
        json!({
            "code": -32000,
            "message": "Frame with the given id does not belong to the target."
        })
    );

    let describe_node = complete_command_dispatch_without_legacy_fallback_for_test(
        &mut ctx,
        json!({
            "id": 9,
            "method": "DOM.describeNode",
            "params": { "backendNodeId": target_backend_node_id, "depth": 1 }
        }),
        "DOM.describeNode node reference without child frame owners should not use legacy fallback",
    )
    .await;
    assert_eq!(
        describe_node[0]["result"]["node"]["nodeName"],
        json!("SECTION")
    );
    let described_backend_node_id = describe_node[0]["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("described backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(described_backend_node_id),
        "DOM.describeNode should return renderer backend ids: {describe_node:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_frame_selector_flow_merges_root_and_closed_shadow_results_in_dom_order() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div id='before'></div><div id='host'></div><div id='after'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<section><div id=\"a\"></div><span><div id=\"b\"></div></span></section>';</script></body></html>",
    )
    .await;

    let main_context_id = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2).await;
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

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": { "expression": "document", "contextId": main_context_id }
    }))
    .await;
    let document_object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!document_object_id.is_empty());

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": document_object_id,
            "functionDeclaration": "function() { return Array.from(this.querySelectorAll('div')); }"
        }
    }))
    .await;
    let root_array_object_id = take_response_by_id(&mut ctx, 5)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!root_array_object_id.is_empty());

    ctx.process_async(
        json!({"id": 6, "method": "DOM.getDocument", "params": { "pierce": true, "depth": -1 }}),
    )
    .await;
    let described_document = take_response_by_id(&mut ctx, 6)["result"]["root"].clone();
    let closed_shadow_root_backend_ids =
        patchright_collect_closed_shadow_root_backend_ids(&described_document);
    assert_eq!(closed_shadow_root_backend_ids.len(), 1);

    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": closed_shadow_root_backend_ids[0],
            "contextId": isolated_context_id
        }
    }))
    .await;
    let shadow_root_object_id = take_response_by_id(&mut ctx, 7)["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!shadow_root_object_id.is_empty());

    ctx.process_async(json!({
        "id": 8,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": shadow_root_object_id,
            "functionDeclaration": "function() { return Array.from(this.querySelectorAll('div')); }"
        }
    }))
    .await;
    let shadow_array_object_id = take_response_by_id(&mut ctx, 8)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!shadow_array_object_id.is_empty());

    let mut current_round = Vec::new();
    for (id, array_object_id, owner_object_id, length) in [
        (9_u64, &root_array_object_id, &document_object_id, 3_u64),
        (
            10_u64,
            &shadow_array_object_id,
            &shadow_root_object_id,
            2_u64,
        ),
    ] {
        for index in 0..length {
            ctx.process_async(json!({
                "id": id + index,
                "method": "Runtime.callFunctionOn",
                "params": {
                    "objectId": owner_object_id,
                    "arguments": [
                        { "value": index },
                        { "objectId": array_object_id }
                    ],
                    "functionDeclaration": "function(i, elems) { return elems[i]; }"
                }
            }))
            .await;
            let object_id =
                take_response_by_id(&mut ctx, id + index)["result"]["result"]["objectId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
            assert!(!object_id.is_empty());
            current_round.push(object_id);
        }
    }

    let mut ordered = Vec::new();
    for (offset, object_id) in current_round.into_iter().enumerate() {
        let id = 20_u64 + offset as u64;
        ctx.process_async(json!({
            "id": id,
            "method": "DOM.describeNode",
            "params": { "objectId": object_id, "depth": -1 }
        }))
        .await;
        let described = take_response_by_id(&mut ctx, id)["result"]["node"].clone();
        let backend_node_id = described["backendNodeId"].as_u64().unwrap_or(0);
        assert!(backend_node_id > 0);
        let node_position =
            patchright_dom_position_for_backend_node_id(&described_document, backend_node_id)
                .unwrap_or_default();
        assert!(!node_position.is_empty());
        ordered.push((
            backend_node_id,
            node_position,
            patchright_element_id_attr(&described),
        ));
    }

    ordered.sort_by(|left, right| {
        patchright_position_sort_key(&left.1).cmp(&patchright_position_sort_key(&right.1))
    });

    let deduped_ids = ordered
        .iter()
        .map(|(_, _, id)| id.clone())
        .collect::<Vec<_>>();
    assert_eq!(deduped_ids, vec!["before", "host", "a", "b", "after"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_owner_rejects_loaded_main_frame_without_owner_element() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getFrameOwner",
        "params": { "frameId": "TID-1" }
    }))
    .await;
    ctx.expect_error(
        2,
        -32000,
        "Frame with the given id does not belong to the target.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_owner_rejects_foreign_frame() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "DOM.getFrameOwner",
        "params": { "frameId": "TID-OTHER" }
    }))
    .await;
    ctx.expect_error(
        2,
        -32000,
        "Frame with the given id does not belong to the target.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_owner_returns_iframe_node_for_child_frame() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<p>child</p>\"></iframe></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.getFrameTree"
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 2);
    let child_frame_id = frame_tree["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();

    let owner_raw = json!({
        "id": 3,
        "method": "DOM.getFrameOwner",
        "params": { "frameId": child_frame_id }
    })
    .to_string();
    let owner_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&owner_raw)
        .expect("DOM.getFrameOwner for a child frame should enter command task path");
    let owner_messages = complete_pending_command_task_for_test(&mut ctx, owner_pending).await;
    let owner = owner_messages
        .iter()
        .find(|message| message["id"] == json!(3))
        .unwrap_or_else(|| {
            panic!("pending DOM.getFrameOwner should produce a response: {owner_messages:?}")
        });
    let owner_node_id = owner["result"]["nodeId"].as_u64().expect("owner node id");
    let owner_backend_node_id = owner["result"]["backendNodeId"]
        .as_u64()
        .expect("owner backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(
            u32::try_from(owner_backend_node_id).expect("owner backend id should fit u32")
        ),
        "DOM.getFrameOwner child-frame backend id should be renderer-owned: {owner}"
    );
    assert_ne!(owner_node_id, owner_backend_node_id);

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": owner_node_id, "depth": 0 }
    }))
    .await;
    let described_owner = take_response_by_id(&mut ctx, 4);
    assert_eq!(
        described_owner["result"]["node"]["nodeName"],
        json!("IFRAME")
    );
    assert_eq!(
        described_owner["result"]["node"]["backendNodeId"],
        json!(owner_backend_node_id),
        "DOM.getFrameOwner should return the same renderer backend id as DOM.describeNode for the owner element: {described_owner:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn async_dispatch_get_document_and_get_frame_owner_surface_child_frame_metadata() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A-ASYNC");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<p>child</p>\"></iframe><div id='peer'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.getFrameTree"
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 2);
    let child_frame_id = frame_tree["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getFrameOwner",
        "params": { "frameId": child_frame_id }
    }))
    .await;
    let owner = take_response_by_id(&mut ctx, 3);
    let owner_node_id = owner["result"]["nodeId"].as_u64().expect("owner node id");
    let owner_backend_node_id = owner["result"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("owner backend node id");
    assert!(
        moli_core::page::is_renderer_backend_node_id(owner_backend_node_id),
        "async DOM.getFrameOwner child-frame backend id should be renderer-owned: {owner}"
    );
    assert_ne!(
        owner["result"]["backendNodeId"],
        json!(owner_node_id),
        "backendNodeId should not fall back to frontend nodeId"
    );

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "nodeId": owner_node_id, "depth": 0 }
    }))
    .await;
    let described_owner = take_response_by_id(&mut ctx, 4);
    assert_eq!(
        described_owner["result"]["node"]["nodeName"],
        json!("IFRAME")
    );

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 5);
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let iframe = child_element_by_node_name(body, "IFRAME");
    let peer_div = child_element_by_node_name(body, "DIV");

    assert_eq!(
        iframe["frameId"],
        frame_tree["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
    );
    assert!(peer_div.get("frameId").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_neutral_query_selector_targets_child_frame_context() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A-CHILD-QUERY");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<main id='inside-frame'>child</main>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;

    let result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverClassic,
                    session_id: None,
                    target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
                    browser_context_id: None,
                },
                root: None,
                selector: "#inside-frame".to_owned(),
                multiple: false,
            },
        ))
        .await
        .expect("child frame query selector should run");

    let DevToolsCommandResult::QuerySelector(result) = result else {
        panic!("expected query selector result");
    };
    let node_id = result.node_ids[0];
    assert!(node_id > 0);

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverClassic,
                session_id: None,
                target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
                browser_context_id: None,
            },
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(node_id)),
            depth: 0,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame queried node describe should run");
    let DevToolsCommandResult::DescribeNode(result) = result else {
        panic!("expected child frame queried node describe result");
    };
    assert_eq!(result.node["nodeName"], json!("MAIN"));
    let backend_node_id = result.node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("queried child frame node should have backend id: {result:?}"));
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "child frame querySelector should bind queried nodes to renderer backend ids: {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_describe_node_uses_the_calling_sessions_whitespace_projection() {
    fn whitespace_text_node_count(node: &Value) -> usize {
        let current = usize::from(
            node["nodeName"] == json!("#text")
                && node["nodeValue"]
                    .as_str()
                    .is_some_and(|value| value.trim().is_empty()),
        );
        current
            + node["children"]
                .as_array()
                .into_iter()
                .flatten()
                .map(whitespace_text_node_count)
                .sum::<usize>()
    }

    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-CHILD-DESCRIBE-WHITESPACE");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-whitespace-default".to_owned());
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe srcdoc=\"<!doctype html><html><body>\n  \
         <main id='inside-frame'>child</main>\n</body></html>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;

    ctx.process_async(json!({
        "id": 3,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1" }
    }))
    .await;
    let all_session_id = take_response_by_id(&mut ctx, 3)["result"]["sessionId"]
        .as_str()
        .expect("auxiliary session id")
        .to_owned();
    ctx.sent.clear();

    for (id, session_id, params) in [
        (4, "SID-whitespace-default", json!({})),
        (
            5,
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

    let child_context = |session_id: &str| DevToolsCommandContext {
        protocol: DevToolsProtocol::Cdp,
        session_id: Some(DevToolsSessionId::from(session_id)),
        target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
        browser_context_id: None,
    };
    let default_result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: child_context("SID-whitespace-default"),
            reference: None,
            depth: -1,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("default child frame root describe should run");
    let DevToolsCommandResult::DescribeNode(default_result) = default_result else {
        panic!("expected default child frame describe result");
    };

    let all_result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: child_context(&all_session_id),
            reference: None,
            depth: -1,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("includeWhitespace=all child frame root describe should run");
    let DevToolsCommandResult::DescribeNode(all_result) = all_result else {
        panic!("expected includeWhitespace=all child frame describe result");
    };

    assert_eq!(
        whitespace_text_node_count(&default_result.node),
        0,
        "the default child-frame Inspector projection must omit indentation text: \
         {default_result:?}"
    );
    assert!(
        whitespace_text_node_count(&all_result.node) > 0,
        "the auxiliary session's includeWhitespace=all mode must reach the child-frame root \
         snapshot: {all_result:?}"
    );
    assert_eq!(
        default_result.node["backendNodeId"], all_result.node["backendNodeId"],
        "session projection must not change child document backend identity"
    );

    let root_backend_node_id = all_result.node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("child document root backend node id");
    let all_referenced = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: child_context(&all_session_id),
            reference: Some(DevToolsDomNodeReference::BackendNodeId(
                root_backend_node_id,
            )),
            depth: -1,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("includeWhitespace=all child root backend describe should run");
    let DevToolsCommandResult::DescribeNode(all_referenced) = all_referenced else {
        panic!("expected referenced child frame describe result");
    };
    assert!(
        whitespace_text_node_count(&all_referenced.node) > 0,
        "referenced child-frame DescribeNode must retain the calling session projection: \
         {all_referenced:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn child_frame_get_outer_html_preserves_session_and_shadow_inclusion() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-CHILD-OUTER-HTML-SHADOW");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-shadow-primary".to_owned());
    navigate_to_data_html_async(
        &mut ctx,
        1,
        concat!(
            "<!doctype html><html><body><iframe srcdoc=\"",
            "<!doctype html><html><body>",
            "<x-child id='host'><template shadowrootmode='closed'>",
            "<span>shadow</span></template>light</x-child>",
            "</body></html>\"></iframe></body></html>"
        ),
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;

    ctx.process_async(json!({
        "id": 3,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1" }
    }))
    .await;
    let auxiliary_session_id = take_response_by_id(&mut ctx, 3)["result"]["sessionId"]
        .as_str()
        .expect("auxiliary session id")
        .to_owned();
    let cdp_context = DevToolsCommandContext {
        protocol: DevToolsProtocol::Cdp,
        session_id: Some(DevToolsSessionId::from(auxiliary_session_id.as_str())),
        target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
        browser_context_id: None,
    };

    let described = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: cdp_context.clone(),
            reference: None,
            depth: -1,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("auxiliary-session child document describe should run");
    let DevToolsCommandResult::DescribeNode(described) = described else {
        panic!("expected child describe result");
    };
    let host = node_tree_element_by_attribute(&described.node, "id", "host")
        .expect("child shadow host snapshot");
    let host_node_id = host["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("auxiliary-session child frontend node id");
    let host_backend_node_id = host["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("child host backend node id");

    let ordinary = "<x-child id=\"host\">light</x-child>";
    let including_shadow = concat!(
        "<x-child id=\"host\"><template shadowrootmode=\"closed\">",
        "<span>shadow</span></template>light</x-child>"
    );
    for (reference, include_shadow_dom, expected) in [
        (
            DevToolsDomNodeReference::FrontendNodeId(host_node_id),
            true,
            including_shadow,
        ),
        (
            DevToolsDomNodeReference::BackendNodeId(host_backend_node_id),
            true,
            including_shadow,
        ),
        (
            DevToolsDomNodeReference::FrontendNodeId(host_node_id),
            false,
            ordinary,
        ),
    ] {
        let result = ctx
            .conn
            .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
                context: cdp_context.clone(),
                reference: Some(reference),
                include_shadow_dom,
            }))
            .await
            .into_parts()
            .0
            .expect("child host outerHTML should run");
        let DevToolsCommandResult::GetOuterHtml(result) = result else {
            panic!("expected child outerHTML result");
        };
        assert_eq!(result.outer_html, expected);
    }

    let cdp_document = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: cdp_context,
            reference: None,
            include_shadow_dom: true,
        }))
        .await
        .into_parts()
        .0
        .expect("child document shadow-inclusive outerHTML should run");
    let DevToolsCommandResult::GetOuterHtml(cdp_document) = cdp_document else {
        panic!("expected child document outerHTML result");
    };
    assert!(cdp_document.outer_html.contains(including_shadow));

    let classic_document = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverClassic,
                session_id: Some(DevToolsSessionId::from("SID-not-an-inspector-session")),
                target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
                browser_context_id: None,
            },
            reference: None,
            include_shadow_dom: false,
        }))
        .await
        .into_parts()
        .0
        .expect("classic child frame page source should keep owner routing");
    let DevToolsCommandResult::GetOuterHtml(classic_document) = classic_document else {
        panic!("expected classic child document outerHTML result");
    };
    assert!(classic_document.outer_html.contains(ordinary));
    assert!(!classic_document.outer_html.contains("shadowrootmode"));
    assert!(!classic_document.outer_html.contains("shadow"));
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_neutral_child_frame_frontend_node_geometry_reads_live_renderer_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A-CHILD-GEOMETRY-LOW");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<main id='inside-frame'>child</main>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: None,
        target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
        browser_context_id: None,
    };

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: context.clone(),
                root: None,
                selector: "#inside-frame".to_owned(),
                multiple: false,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("child frame query selector should run");

    let DevToolsCommandResult::QuerySelector(result) = result else {
        panic!("expected query selector result");
    };
    let node_id = result.node_ids[0];

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DomGeometry(DevToolsDomGeometryCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::FrontendNodeId(node_id),
            operation: DevToolsDomGeometryOperation::GetBoxModel,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame frontend node geometry should run");
    let DevToolsCommandResult::DomGeometry(result) = result else {
        panic!("expected child frontend node geometry result");
    };
    assert_eq!(
        result
            .box_model
            .as_ref()
            .map(|model| model.border.points.len()),
        Some(8)
    );
    assert!(result.quads.is_empty());

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DomGeometry(DevToolsDomGeometryCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::FrontendNodeId(node_id),
            operation: DevToolsDomGeometryOperation::GetContentQuads,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame frontend node content quads should run");
    let DevToolsCommandResult::DomGeometry(result) = result else {
        panic!("expected child frontend node content quads result");
    };
    assert_eq!(result.quads.first().map(|quad| quad.points.len()), Some(8));
    assert!(result.box_model.is_none());

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::ScrollIntoViewIfNeeded(
            DevToolsScrollIntoViewIfNeededCommand {
                context,
                reference: Some(DevToolsDomNodeReference::FrontendNodeId(node_id)),
                rect: None,
            },
        ))
        .await
        .into_parts()
        .0
        .expect("child frame frontend node scrollIntoViewIfNeeded should run");
    assert!(matches!(result, DevToolsCommandResult::Empty));
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_neutral_resolve_node_targets_child_frame_context() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A-CHILD-RESOLVE-SHARED");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1".to_owned());
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><main id='top' style='display:flex'>top</main><iframe id='child' srcdoc=\"<main id='inside-frame' style='display:grid'>child</main>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("SID-1")),
        target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
        browser_context_id: None,
    };

    let result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: context.clone(),
                root: None,
                selector: "#inside-frame".to_owned(),
                multiple: false,
            },
        ))
        .await
        .expect("child frame query selector should run");

    let DevToolsCommandResult::QuerySelector(result) = result else {
        panic!("expected query selector result");
    };
    let node_id = result.node_ids[0];

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(node_id)),
            depth: 0,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame frontend node describe should run");
    let DevToolsCommandResult::DescribeNode(result) = result else {
        panic!("expected child frontend node describe result");
    };
    assert_eq!(result.node["nodeName"], json!("MAIN"));
    assert_eq!(result.node["attributes"][1], json!("inside-frame"));

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetAttributes(
            DevToolsGetAttributesCommand {
                context: context.clone(),
                reference: DevToolsDomNodeReference::FrontendNodeId(node_id),
            },
        ))
        .await
        .into_parts()
        .0
        .expect("child frame frontend node attributes should run");
    let DevToolsCommandResult::GetAttributes(result) = result else {
        panic!("expected child frontend node attributes result");
    };
    assert!(
        result
            .attributes
            .iter()
            .any(|attribute| attribute.name == "id" && attribute.value == "inside-frame")
    );
    assert!(
        result
            .attributes
            .iter()
            .any(|attribute| attribute.name == "style" && attribute.value == "display:grid")
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetText(DevToolsGetTextCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::FrontendNodeId(node_id),
        }))
        .await
        .into_parts()
        .0
        .expect("child frame frontend node text should run");
    let DevToolsCommandResult::GetText(result) = result else {
        panic!("expected child frontend node text result");
    };
    assert_eq!(result.text, "child");

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetProperty(DevToolsGetPropertyCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::FrontendNodeId(node_id),
            name: "id".to_owned(),
        }))
        .await
        .into_parts()
        .0
        .expect("child frame frontend node property should run");
    let DevToolsCommandResult::GetProperty(result) = result else {
        panic!("expected child frontend node property result");
    };
    assert_eq!(result.value, json!("inside-frame"));

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(node_id)),
            include_shadow_dom: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame frontend node outerHTML should run");
    let DevToolsCommandResult::GetOuterHtml(result) = result else {
        panic!("expected child frontend node outerHTML result");
    };
    assert_eq!(
        result.outer_html,
        "<main id=\"inside-frame\" style=\"display:grid\">child</main>"
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: context.clone(),
            reference: None,
            include_shadow_dom: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame root outerHTML should run");
    let DevToolsCommandResult::GetOuterHtml(result) = result else {
        panic!("expected child frame root outerHTML result");
    };
    assert!(
        result.outer_html.contains("inside-frame"),
        "child frame root outerHTML should serialize child document: {:?}",
        result.outer_html
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: context.clone(),
            reference: None,
            depth: 1,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame root describe should run");
    let DevToolsCommandResult::DescribeNode(result) = result else {
        panic!("expected child frame root describe node result");
    };
    assert_eq!(result.node["nodeName"], json!("#document"));
    assert_eq!(result.node["children"][0]["nodeName"], json!("HTML"));
    let root_backend_node_id = result.node["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| panic!("child frame root should have backend id: {result:?}"));
    assert!(
        moli_core::page::is_renderer_backend_node_id(root_backend_node_id),
        "child frame root describe should expose renderer-owned backend id: {result:?}"
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::BackendNodeId(
                root_backend_node_id,
            )),
            depth: 1,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame root backend describe should run");
    let DevToolsCommandResult::DescribeNode(result) = result else {
        panic!("expected child frame root backend describe node result");
    };
    assert_eq!(result.node["nodeName"], json!("#document"));
    assert_eq!(result.node["backendNodeId"], json!(root_backend_node_id));
    assert_eq!(result.node["children"][0]["nodeName"], json!("HTML"));

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::ResolveNode(DevToolsResolveNodeCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::FrontendNodeId(node_id),
            execution_context_id: None,
            object_group: Some("child-frame-resolve".to_owned()),
        }))
        .await
        .into_parts()
        .0
        .expect("child frame resolve node should run");

    let DevToolsCommandResult::ResolveNode(result) = result else {
        panic!("expected resolve node result");
    };
    let object_id = result.object["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("expected child frame node object id: {:?}", result.object));

    let result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::CallFunction(
            DevToolsCallFunctionCommand {
                context: context.clone(),
                realm_id: None,
                world_name: None,
                object_id: Some(DevToolsRemoteHandleId::from(object_id.clone())),
                this_parameter: None,
                function_declaration:
                    "function() { return this.id + ':' + getComputedStyle(this).display; }"
                        .to_owned(),
                arguments: Vec::new(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                object_group: None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await
        .expect("child frame callFunctionOn should run");

    let DevToolsCommandResult::Script(result) = result else {
        panic!("expected script result");
    };
    let DevToolsScriptResult::Value(value) = *result else {
        panic!("expected script value result");
    };
    assert_eq!(value.value, json!("inside-frame:grid"));

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.describeNode",
        "params": { "objectId": object_id, "depth": 0 }
    }))
    .await;
    let object_describe = take_response_by_id(&mut ctx, 4);
    let renderer_backend_node_id = object_describe["result"]["node"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| {
            panic!("child object describe should return backend id: {object_describe}")
        });
    assert!(
        moli_core::page::is_renderer_backend_node_id(renderer_backend_node_id),
        "child object describe should return renderer-owned backend id: {object_describe}"
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::BackendNodeId(
                renderer_backend_node_id,
            )),
            depth: 0,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame high backend describe should run");
    let DevToolsCommandResult::DescribeNode(result) = result else {
        panic!("expected child high backend describe node result");
    };
    assert_eq!(result.node["nodeName"], json!("MAIN"));
    assert_eq!(
        result.node["backendNodeId"],
        json!(renderer_backend_node_id)
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetAttributes(
            DevToolsGetAttributesCommand {
                context: context.clone(),
                reference: DevToolsDomNodeReference::BackendNodeId(renderer_backend_node_id),
            },
        ))
        .await
        .into_parts()
        .0
        .expect("child frame high backend attributes should run");
    let DevToolsCommandResult::GetAttributes(result) = result else {
        panic!("expected child high backend attributes result");
    };
    assert!(
        result
            .attributes
            .iter()
            .any(|attribute| attribute.name == "id" && attribute.value == "inside-frame")
    );
    assert!(
        result
            .attributes
            .iter()
            .any(|attribute| attribute.name == "style" && attribute.value == "display:grid")
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetText(DevToolsGetTextCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::BackendNodeId(renderer_backend_node_id),
        }))
        .await
        .into_parts()
        .0
        .expect("child frame high backend text should run");
    let DevToolsCommandResult::GetText(result) = result else {
        panic!("expected child high backend text result");
    };
    assert_eq!(result.text, "child");

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetProperty(DevToolsGetPropertyCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::BackendNodeId(renderer_backend_node_id),
            name: "id".to_owned(),
        }))
        .await
        .into_parts()
        .0
        .expect("child frame high backend property should run");
    let DevToolsCommandResult::GetProperty(result) = result else {
        panic!("expected child high backend property result");
    };
    assert_eq!(result.value, json!("inside-frame"));

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::BackendNodeId(
                renderer_backend_node_id,
            )),
            include_shadow_dom: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame high backend outerHTML should run");
    let DevToolsCommandResult::GetOuterHtml(result) = result else {
        panic!("expected child high backend outerHTML result");
    };
    assert_eq!(
        result.outer_html,
        "<main id=\"inside-frame\" style=\"display:grid\">child</main>"
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::ResolveNode(DevToolsResolveNodeCommand {
            context: context.clone(),
            reference: DevToolsDomNodeReference::BackendNodeId(renderer_backend_node_id),
            execution_context_id: None,
            object_group: Some("child-frame-high-backend-resolve".to_owned()),
        }))
        .await
        .into_parts()
        .0
        .expect("child frame high backend resolve should run");
    let DevToolsCommandResult::ResolveNode(result) = result else {
        panic!("expected child high backend resolve node result");
    };
    let high_backend_object_id = result.object["objectId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            panic!(
                "expected child high backend resolve object id: {:?}",
                result.object
            )
        });

    let result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::CallFunction(
            DevToolsCallFunctionCommand {
                context: context.clone(),
                realm_id: None,
                world_name: None,
                object_id: Some(DevToolsRemoteHandleId::from(high_backend_object_id)),
                this_parameter: None,
                function_declaration:
                    "function() { return this.id + ':' + getComputedStyle(this).display; }"
                        .to_owned(),
                arguments: Vec::new(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                object_group: None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await
        .expect("child frame high backend callFunctionOn should run");
    let DevToolsCommandResult::Script(result) = result else {
        panic!("expected script result");
    };
    let DevToolsScriptResult::Value(value) = *result else {
        panic!("expected script value result");
    };
    assert_eq!(value.value, json!("inside-frame:grid"));

    let result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::DomGeometry(
            DevToolsDomGeometryCommand {
                context: context.clone(),
                reference: DevToolsDomNodeReference::BackendNodeId(renderer_backend_node_id),
                operation: DevToolsDomGeometryOperation::GetBoxModel,
            },
        ))
        .await
        .expect("child frame high backend geometry should run");
    let DevToolsCommandResult::DomGeometry(result) = result else {
        panic!("expected child high backend geometry result");
    };
    assert_eq!(
        result
            .box_model
            .as_ref()
            .map(|model| model.border.points.len()),
        Some(8)
    );
    assert!(result.quads.is_empty());

    let result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(
            DevToolsCommand::ScrollIntoViewIfNeeded(DevToolsScrollIntoViewIfNeededCommand {
                context,
                reference: Some(DevToolsDomNodeReference::BackendNodeId(
                    renderer_backend_node_id,
                )),
                rect: None,
            }),
        )
        .await
        .expect("child frame high backend scrollIntoViewIfNeeded should run");
    assert!(matches!(result, DevToolsCommandResult::Empty));
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_neutral_describe_node_targets_child_frame_with_pierce() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A-CHILD-DESCRIBE-SHARED");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<span id=inside>child shadow</span>';</script>\"></iframe></body></html>",
    )
    .await;
    let child_frame_id = child_frame_id_for_single_iframe_async(&mut ctx, 2).await;
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: None,
        target_id: Some(DevToolsTargetId::from(child_frame_id.as_str())),
        browser_context_id: None,
    };

    let result = ctx
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
        .expect("child frame query selector should run");
    let DevToolsCommandResult::QuerySelector(result) = result else {
        panic!("expected query selector result");
    };
    let host_node_id = result.node_ids[0];

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: context.clone(),
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(host_node_id)),
            depth: -1,
            pierce: true,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame describe node should run");
    let DevToolsCommandResult::DescribeNode(result) = result else {
        panic!("expected describe node result");
    };
    let shadow_roots = result.node["shadowRoots"]
        .as_array()
        .unwrap_or_else(|| panic!("child frame host should expose closed shadow root: {result:?}"));
    assert_eq!(shadow_roots.len(), 1);
    assert_eq!(shadow_roots[0]["shadowRootType"], json!("closed"));
    let shadow_root_node_id = shadow_roots[0]["nodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .expect("child-frame shadow root should have frontend node id");
    let inside = node_array_element_by_attribute(&shadow_roots[0]["children"], "id", "inside");
    assert!(
        inside.is_some(),
        "pierced child-frame DescribeNode should include closed shadow contents: {result:?}"
    );

    let result = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context,
            reference: Some(DevToolsDomNodeReference::FrontendNodeId(
                shadow_root_node_id,
            )),
            depth: 0,
            pierce: false,
        }))
        .await
        .into_parts()
        .0
        .expect("child frame shadow root describe node should run");
    let DevToolsCommandResult::DescribeNode(result) = result else {
        panic!("expected shadow root describe node result");
    };
    assert_eq!(
        result.node["shadowRootType"],
        json!("closed"),
        "expected shadow root frontend id to describe the shadow root node: {:?}",
        result.node
    );
    assert_eq!(result.node["parentId"], json!(host_node_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn describe_node_surfaces_child_frame_id_for_iframe_owner() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<p>child</p>\"></iframe></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "Page.getFrameTree"}))
        .await;
    let child_frame_id = take_response_by_id(&mut ctx, 2)["result"]["frameTree"]["childFrames"][0]
        ["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();

    ctx.process_async(json!({"id": 3, "method": "DOM.getDocument"}))
        .await;
    let root_id = take_response_by_id(&mut ctx, 3)["result"]["root"]["nodeId"]
        .as_u64()
        .expect("root node id");

    ctx.process_async(json!({
        "id": 4,
        "method": "DOM.querySelector",
        "params": { "nodeId": root_id, "selector": "#child" }
    }))
    .await;
    let iframe_node_id = take_query_selector_node_id(&mut ctx, 4);

    let describe_raw = json!({
        "id": 5,
        "method": "DOM.describeNode",
        "params": { "nodeId": iframe_node_id, "depth": 0 }
    })
    .to_string();
    let describe_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&describe_raw)
        .expect("DOM.describeNode for an iframe owner should enter command task path");
    let describe_messages =
        complete_pending_command_task_for_test(&mut ctx, describe_pending).await;
    let described_iframe = describe_messages
        .iter()
        .find(|message| message["id"] == json!(5))
        .unwrap_or_else(|| {
            panic!(
                "pending iframe DOM.describeNode should produce a response: {describe_messages:?}"
            )
        });
    assert_eq!(
        described_iframe["result"]["node"]["nodeName"],
        json!("IFRAME")
    );
    assert_eq!(
        described_iframe["result"]["node"]["frameId"],
        json!(child_frame_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_surfaces_child_frame_id_for_iframe_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<p>child</p>\"></iframe><div id='peer'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "Page.getFrameTree"}))
        .await;
    let child_frame_id = take_response_by_id(&mut ctx, 2)["result"]["frameTree"]["childFrames"][0]
        ["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();

    let get_document_raw = json!({
        "id": 3,
        "method": "DOM.getDocument",
        "params": { "depth": -1 }
    })
    .to_string();
    let get_document_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&get_document_raw)
        .expect("DOM.getDocument with child frames should start as a pending command");
    let get_document_messages =
        complete_pending_command_task_for_test(&mut ctx, get_document_pending).await;
    let document = get_document_messages
        .iter()
        .find(|message| message["id"] == json!(3))
        .expect("DOM.getDocument response");
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let iframe = child_element_by_node_name(body, "IFRAME");
    let peer_div = child_element_by_node_name(body, "DIV");

    assert_eq!(iframe["frameId"], json!(child_frame_id));
    assert!(
        iframe.get("contentDocument").is_none(),
        "contentDocument requires pierce=true: {iframe}"
    );
    assert!(peer_div.get("frameId").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_with_pierce_projects_child_frame_content_document() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<!doctype html><html><body><template id='child-template'><span>payload</span></template><p id='child-text'>child</p></body></html>\"></iframe></body></html>",
    )
    .await;

    let get_document_raw = json!({
        "id": 2,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    })
    .to_string();
    let get_document_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&get_document_raw)
        .expect("DOM.getDocument with child frames should start as a pending command");
    let get_document_messages =
        complete_pending_command_task_for_test(&mut ctx, get_document_pending).await;
    let document = get_document_messages
        .iter()
        .find(|message| message["id"] == json!(2))
        .expect("DOM.getDocument response");
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let iframe = child_element_by_node_name(body, "IFRAME");
    let content_document = &iframe["contentDocument"];

    assert_eq!(content_document["nodeType"], json!(9));
    assert_eq!(content_document["nodeName"], json!("#document"));
    assert!(
        content_document.get("parentId").is_none(),
        "contentDocument is associated with the frame owner, not its DOM child: {content_document}"
    );
    let child_html = child_element_by_node_name(content_document, "HTML");
    let child_body = child_element_by_node_name(child_html, "BODY");
    let template = child_element_by_node_name(child_body, "TEMPLATE");
    assert_eq!(template["templateContent"]["nodeType"], json!(11));
    let paragraph = child_element_by_node_name(child_body, "P");
    assert_eq!(paragraph["attributes"], json!(["id", "child-text"]));
    assert_eq!(paragraph["children"][0]["nodeValue"], json!("child"));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_document_with_pierce_omits_opaque_sandbox_content_document() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        201,
        "<!doctype html><html><body><iframe id='opaque' sandbox='allow-scripts' srcdoc=\"<p id='opaque-child'>child</p>\"></iframe></body></html>",
    )
    .await;

    let get_document_raw = json!({
        "id": 202,
        "method": "DOM.getDocument",
        "params": { "depth": -1, "pierce": true }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&get_document_raw)
        .expect("opaque-frame DOM.getDocument should start as a pending command");
    let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;
    let document = messages
        .iter()
        .find(|message| message["id"] == json!(202))
        .expect("DOM.getDocument response");
    let html = child_element_by_node_name(&document["result"]["root"], "HTML");
    let body = child_element_by_node_name(html, "BODY");
    let iframe = child_element_by_node_name(body, "IFRAME");

    assert!(iframe["frameId"].is_string());
    assert!(
        iframe.get("contentDocument").is_none(),
        "opaque sandbox frame belongs to a separate Chromium target boundary: {iframe}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn request_child_nodes_surfaces_child_frame_id_for_iframe_node() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><iframe id='child' srcdoc=\"<p>child</p>\"></iframe><div id='peer'></div></body></html>",
    )
    .await;

    ctx.process_async(json!({"id": 2, "method": "Page.getFrameTree"}))
        .await;
    let child_frame_id = take_response_by_id(&mut ctx, 2)["result"]["frameTree"]["childFrames"][0]
        ["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();

    ctx.process_async(json!({
        "id": 3,
        "method": "DOM.getDocument",
        "params": { "depth": 1 }
    }))
    .await;
    let document = take_response_by_id(&mut ctx, 3);
    let html_node_id = child_element_by_node_name(&document["result"]["root"], "HTML")["nodeId"]
        .as_u64()
        .expect("html node id");

    let request_child_nodes_raw = json!({
        "id": 4,
        "method": "DOM.requestChildNodes",
        "params": { "nodeId": html_node_id, "depth": -1 }
    })
    .to_string();
    let request_child_nodes_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&request_child_nodes_raw)
        .expect("DOM.requestChildNodes with child frames should start as a pending command");
    let request_child_nodes_messages =
        complete_pending_command_task_for_test(&mut ctx, request_child_nodes_pending).await;
    let set_child_nodes = request_child_nodes_messages
        .iter()
        .find(|message| message["method"] == json!("DOM.setChildNodes"))
        .cloned()
        .expect("DOM.setChildNodes event");
    let body = set_child_nodes["params"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["nodeName"] == json!("BODY")))
        .expect("BODY node");
    let iframe = child_element_by_node_name(body, "IFRAME");
    let peer_div = child_element_by_node_name(body, "DIV");

    assert_eq!(iframe["frameId"], json!(child_frame_id));
    assert!(peer_div.get("frameId").is_none());
    let response = request_child_nodes_messages
        .iter()
        .find(|message| message["id"] == json!(4))
        .expect("DOM.requestChildNodes response");
    assert_eq!(response["result"], json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_owner_without_browser_context_errors() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 1,
        "method": "DOM.getFrameOwner",
        "params": { "frameId": "TID-1" }
    }))
    .await;
    ctx.expect_error(1, -31998, "BrowserContextNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_owner_without_loaded_page_errors() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    if let Some(bc) = ctx.conn.browser_context.as_mut() {
        bc.set_active_target_id("TID-1");
    }

    ctx.process_async(json!({
        "id": 1,
        "method": "DOM.getFrameOwner",
        "params": { "frameId": "TID-1" }
    }))
    .await;
    ctx.expect_error(
        1,
        -32000,
        "Frame with the given id does not belong to the target.",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_owner_invalid_params_error() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");

    ctx.process_async(json!({
        "id": 1,
        "method": "DOM.getFrameOwner"
    }))
    .await;
    ctx.expect_error(1, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_frame_owner_without_selected_target_errors() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(crate::conn::BrowserContext::new("BID-A".to_owned()));

    ctx.process_async(json!({
        "id": 1,
        "method": "DOM.getFrameOwner",
        "params": { "frameId": "TID-1" }
    }))
    .await;
    ctx.expect_error(1, -31998, "BrowserContextNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_frame_selector_or_style_merge_deduplicates_by_backend_node_id_in_dom_order() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div class='target' id='before'></div><div class='target' id='host'></div><div class='target' id='after'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<section><div class=\"target\" id=\"a\"></div><span><div class=\"target\" id=\"b\"></div></span></section>';</script></body></html>",
    )
    .await;

    let main_context_id = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": "TID-1",
            "worldName": "patchright-utility"
        }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    assert!(isolated_context_id > 0);
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document",
            "contextId": main_context_id
        }
    }))
    .await;
    let document_object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!document_object_id.is_empty());

    ctx.process_async(json!({
            "id": 5,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": document_object_id,
                "functionDeclaration": "function() { return Array.from(this.querySelectorAll('.target')); }"
            }
        })).await;
    let current_array_object_id = take_response_by_id(&mut ctx, 5)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!current_array_object_id.is_empty());

    ctx.process_async(json!({
            "id": 6,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": document_object_id,
                "functionDeclaration": "function() { return Array.from(this.querySelectorAll('#before, #after')); }"
            }
        })).await;
    let orred_array_object_id = take_response_by_id(&mut ctx, 6)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!orred_array_object_id.is_empty());

    ctx.process_async(json!({
        "id": 7,
        "method": "DOM.getDocument",
        "params": { "pierce": true, "depth": -1 }
    }))
    .await;
    let described_document = take_response_by_id(&mut ctx, 7)["result"]["root"].clone();
    let closed_shadow_root_backend_ids =
        patchright_collect_closed_shadow_root_backend_ids(&described_document);
    assert_eq!(closed_shadow_root_backend_ids.len(), 1);

    ctx.process_async(json!({
        "id": 8,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": closed_shadow_root_backend_ids[0],
            "contextId": isolated_context_id
        }
    }))
    .await;
    let shadow_root_object_id = take_response_by_id(&mut ctx, 8)["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!shadow_root_object_id.is_empty());

    ctx.process_async(json!({
            "id": 9,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": shadow_root_object_id,
                "functionDeclaration": "function() { return Array.from(this.querySelectorAll('.target')); }"
            }
        })).await;
    let shadow_array_object_id = take_response_by_id(&mut ctx, 9)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!shadow_array_object_id.is_empty());

    let array_lengths = [
        (10_u64, &current_array_object_id, 3_u64),
        (11_u64, &orred_array_object_id, 2_u64),
        (12_u64, &shadow_array_object_id, 2_u64),
    ];
    for (id, array_object_id, expected_length) in array_lengths {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": array_object_id,
                "returnByValue": true,
                "functionDeclaration": "function() { return this.length; }"
            }
        }))
        .await;
        let length = take_response_by_id(&mut ctx, id)["result"]["result"]["value"]
            .as_u64()
            .unwrap_or_default();
        assert_eq!(length, expected_length);
    }

    let mut selected_object_ids = Vec::new();

    for index in 0..3_u64 {
        ctx.process_async(json!({
            "id": 13,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": document_object_id,
                "arguments": [
                    { "value": index },
                    { "objectId": current_array_object_id }
                ],
                "functionDeclaration": "function(i, elems) { return elems[i]; }"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 13)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!object_id.is_empty());
        selected_object_ids.push(object_id);
    }

    for index in 0..2_u64 {
        ctx.process_async(json!({
            "id": 14,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": document_object_id,
                "arguments": [
                    { "value": index },
                    { "objectId": orred_array_object_id }
                ],
                "functionDeclaration": "function(i, elems) { return elems[i]; }"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 14)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!object_id.is_empty());
        selected_object_ids.push(object_id);
    }

    for index in 0..2_u64 {
        ctx.process_async(json!({
            "id": 15,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": shadow_root_object_id,
                "arguments": [
                    { "value": index },
                    { "objectId": shadow_array_object_id }
                ],
                "functionDeclaration": "function(i, elems) { return elems[i]; }"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 15)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!object_id.is_empty());
        selected_object_ids.push(object_id);
    }

    assert_eq!(selected_object_ids.len(), 7);

    let mut ordered = Vec::new();
    for object_id in selected_object_ids {
        ctx.process_async(json!({
            "id": 16,
            "method": "DOM.describeNode",
            "params": {
                "objectId": object_id,
                "depth": -1
            }
        }))
        .await;
        let described = take_response_by_id(&mut ctx, 16)["result"]["node"].clone();
        let backend_node_id = described["backendNodeId"].as_u64().unwrap_or(0);
        assert!(backend_node_id > 0);
        let node_position =
            patchright_dom_position_for_backend_node_id(&described_document, backend_node_id)
                .unwrap_or_default();
        assert!(!node_position.is_empty());
        ordered.push((
            backend_node_id,
            node_position,
            patchright_element_id_attr(&described),
        ));
    }

    ordered.sort_by(|left, right| {
        patchright_position_sort_key(&left.1).cmp(&patchright_position_sort_key(&right.1))
    });

    let duplicate_count = ordered
        .iter()
        .filter(|(_, _, id)| id == "before" || id == "after")
        .count();
    assert_eq!(duplicate_count, 4);

    let mut seen_backend_node_ids = std::collections::HashSet::new();
    ordered.retain(|(backend_node_id, _, _)| seen_backend_node_ids.insert(*backend_node_id));

    let ordered_ids = ordered.into_iter().map(|(_, _, id)| id).collect::<Vec<_>>();
    assert_eq!(ordered_ids, vec!["before", "host", "a", "b", "after"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_frame_selector_and_style_merge_preserves_current_dom_order_when_intersecting() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div class='target' id='before'></div><div class='target' id='host'></div><div class='target' id='after'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<section><div class=\"target\" id=\"a\"></div><span><div class=\"target\" id=\"b\"></div></span></section>';</script></body></html>",
    )
    .await;

    let main_context_id = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": "TID-1",
            "worldName": "patchright-utility"
        }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    assert!(isolated_context_id > 0);
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document",
            "contextId": main_context_id
        }
    }))
    .await;
    let document_object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!document_object_id.is_empty());

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getDocument",
        "params": { "pierce": true, "depth": -1 }
    }))
    .await;
    let described_document = take_response_by_id(&mut ctx, 5)["result"]["root"].clone();
    let closed_shadow_root_backend_ids =
        patchright_collect_closed_shadow_root_backend_ids(&described_document);
    assert_eq!(closed_shadow_root_backend_ids.len(), 1);

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": closed_shadow_root_backend_ids[0],
            "contextId": isolated_context_id
        }
    }))
    .await;
    let shadow_root_object_id = take_response_by_id(&mut ctx, 6)["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!shadow_root_object_id.is_empty());

    ctx.process_async(json!({
            "id": 7,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": document_object_id,
                "functionDeclaration": "function() { return Array.from(this.querySelectorAll('.target')); }"
            }
        })).await;
    let current_root_array_object_id =
        take_response_by_id(&mut ctx, 7)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
    assert!(!current_root_array_object_id.is_empty());

    ctx.process_async(json!({
            "id": 8,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": shadow_root_object_id,
                "functionDeclaration": "function() { return Array.from(this.querySelectorAll('.target')); }"
            }
        })).await;
    let current_shadow_array_object_id =
        take_response_by_id(&mut ctx, 8)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
    assert!(!current_shadow_array_object_id.is_empty());

    ctx.process_async(json!({
            "id": 9,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": document_object_id,
                "functionDeclaration": "function() { return Array.from(this.querySelectorAll('#after, #a')); }"
            }
        })).await;
    let anded_root_array_object_id =
        take_response_by_id(&mut ctx, 9)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
    assert!(!anded_root_array_object_id.is_empty());

    ctx.process_async(json!({
            "id": 10,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": shadow_root_object_id,
                "functionDeclaration": "function() { return Array.from(this.querySelectorAll('#after, #a')); }"
            }
        })).await;
    let anded_shadow_array_object_id =
        take_response_by_id(&mut ctx, 10)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
    assert!(!anded_shadow_array_object_id.is_empty());

    let array_lengths = [
        (11_u64, &current_root_array_object_id, 3_u64),
        (12_u64, &current_shadow_array_object_id, 2_u64),
        (13_u64, &anded_root_array_object_id, 1_u64),
        (14_u64, &anded_shadow_array_object_id, 1_u64),
    ];
    for (id, array_object_id, expected_length) in array_lengths {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": array_object_id,
                "returnByValue": true,
                "functionDeclaration": "function() { return this.length; }"
            }
        }))
        .await;
        let length = take_response_by_id(&mut ctx, id)["result"]["result"]["value"]
            .as_u64()
            .unwrap_or_default();
        assert_eq!(length, expected_length);
    }

    let mut current_round = Vec::new();
    for index in 0..3_u64 {
        ctx.process_async(json!({
            "id": 15,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": document_object_id,
                "arguments": [
                    { "value": index },
                    { "objectId": current_root_array_object_id }
                ],
                "functionDeclaration": "function(i, elems) { return elems[i]; }"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 15)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!object_id.is_empty());
        current_round.push(object_id);
    }

    for index in 0..2_u64 {
        ctx.process_async(json!({
            "id": 16,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": shadow_root_object_id,
                "arguments": [
                    { "value": index },
                    { "objectId": current_shadow_array_object_id }
                ],
                "functionDeclaration": "function(i, elems) { return elems[i]; }"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 16)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!object_id.is_empty());
        current_round.push(object_id);
    }

    let mut current_ordered = Vec::new();
    for object_id in current_round {
        ctx.process_async(json!({
            "id": 17,
            "method": "DOM.describeNode",
            "params": {
                "objectId": object_id,
                "depth": -1
            }
        }))
        .await;
        let described = take_response_by_id(&mut ctx, 17)["result"]["node"].clone();
        let backend_node_id = described["backendNodeId"].as_u64().unwrap_or(0);
        assert!(backend_node_id > 0);
        let node_position =
            patchright_dom_position_for_backend_node_id(&described_document, backend_node_id)
                .unwrap_or_default();
        assert!(!node_position.is_empty());
        current_ordered.push((
            backend_node_id,
            node_position,
            patchright_element_id_attr(&described),
        ));
    }

    current_ordered.sort_by(|left, right| {
        patchright_position_sort_key(&left.1).cmp(&patchright_position_sort_key(&right.1))
    });

    let mut seen_backend_node_ids = std::collections::HashSet::new();
    current_ordered
        .retain(|(backend_node_id, _, _)| seen_backend_node_ids.insert(*backend_node_id));

    let mut anded_backend_node_ids = std::collections::HashSet::new();
    for (id, array_object_id) in [
        (18_u64, &anded_root_array_object_id),
        (19_u64, &anded_shadow_array_object_id),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": array_object_id,
                "arguments": [
                    { "value": 0 },
                    { "objectId": array_object_id }
                ],
                "functionDeclaration": "function(i, elems) { return elems[i]; }"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, id)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!object_id.is_empty());

        ctx.process_async(json!({
            "id": id + 10,
            "method": "DOM.describeNode",
            "params": {
                "objectId": object_id,
                "depth": -1
            }
        }))
        .await;
        let described = take_response_by_id(&mut ctx, id + 10)["result"]["node"].clone();
        let backend_node_id = described["backendNodeId"].as_u64().unwrap_or(0);
        assert!(backend_node_id > 0);
        anded_backend_node_ids.insert(backend_node_id);
    }

    assert_eq!(anded_backend_node_ids.len(), 2);

    let intersected_ids = current_ordered
        .into_iter()
        .filter(|(backend_node_id, _, _)| anded_backend_node_ids.contains(backend_node_id))
        .map(|(_, _, id)| id)
        .collect::<Vec<_>>();
    assert_eq!(intersected_ids, vec!["a", "after"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn patchright_frame_selector_or_then_and_chain_keeps_deduped_dom_order() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-A");
    navigate_to_data_html_async(
        &mut ctx,
        1,
        "<!doctype html><html><body><div class='target' id='before'></div><div class='target' id='host'></div><div class='target' id='after'></div><script>const root=document.getElementById('host').attachShadow({mode:'closed'});root.innerHTML='<section><div class=\"target\" id=\"a\"></div><span><div class=\"target\" id=\"b\"></div></span></section>';</script></body></html>",
    )
    .await;

    let main_context_id = enable_runtime_and_take_execution_context_id_async(&mut ctx, 2).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": "TID-1",
            "worldName": "patchright-utility"
        }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 3)["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    assert!(isolated_context_id > 0);
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document",
            "contextId": main_context_id
        }
    }))
    .await;
    let document_object_id = take_response_by_id(&mut ctx, 4)["result"]["result"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!document_object_id.is_empty());

    ctx.process_async(json!({
        "id": 5,
        "method": "DOM.getDocument",
        "params": { "pierce": true, "depth": -1 }
    }))
    .await;
    let described_document = take_response_by_id(&mut ctx, 5)["result"]["root"].clone();
    let closed_shadow_root_backend_ids =
        patchright_collect_closed_shadow_root_backend_ids(&described_document);
    assert_eq!(closed_shadow_root_backend_ids.len(), 1);

    ctx.process_async(json!({
        "id": 6,
        "method": "DOM.resolveNode",
        "params": {
            "backendNodeId": closed_shadow_root_backend_ids[0],
            "contextId": isolated_context_id
        }
    }))
    .await;
    let shadow_root_object_id = take_response_by_id(&mut ctx, 6)["result"]["object"]["objectId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!shadow_root_object_id.is_empty());

    let array_specs = [
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
    ];
    let mut array_object_ids = Vec::new();
    for (id, function_declaration, object_id) in array_specs {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": function_declaration
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, id)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!object_id.is_empty());
        array_object_ids.push(object_id);
    }

    let expected_lengths = [3_u64, 2, 2, 1, 1];
    for (index, expected_length) in expected_lengths.into_iter().enumerate() {
        let id = 20 + index as u64;
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": array_object_ids[index],
                "returnByValue": true,
                "functionDeclaration": "function() { return this.length; }"
            }
        }))
        .await;
        let length = take_response_by_id(&mut ctx, id)["result"]["result"]["value"]
            .as_u64()
            .unwrap_or_default();
        assert_eq!(length, expected_length);
    }

    let source_arrays = [
        (&document_object_id, &array_object_ids[0], 3_u64, 30_u64),
        (&shadow_root_object_id, &array_object_ids[1], 2_u64, 30_u64),
        (&document_object_id, &array_object_ids[2], 2_u64, 30_u64),
    ];
    let mut current_round = Vec::new();
    for (base_object_id, array_object_id, length, id) in source_arrays {
        for index in 0..length {
            ctx.process_async(json!({
                "id": id,
                "method": "Runtime.callFunctionOn",
                "params": {
                    "objectId": base_object_id,
                    "arguments": [
                        { "value": index },
                        { "objectId": array_object_id }
                    ],
                    "functionDeclaration": "function(i, elems) { return elems[i]; }"
                }
            }))
            .await;
            let object_id = take_response_by_id(&mut ctx, id)["result"]["result"]["objectId"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            assert!(!object_id.is_empty());
            current_round.push(object_id);
        }
    }

    let mut current_ordered = Vec::new();
    for object_id in current_round {
        ctx.process_async(json!({
            "id": 31,
            "method": "DOM.describeNode",
            "params": {
                "objectId": object_id,
                "depth": -1
            }
        }))
        .await;
        let described = take_response_by_id(&mut ctx, 31)["result"]["node"].clone();
        let backend_node_id = described["backendNodeId"].as_u64().unwrap_or(0);
        assert!(backend_node_id > 0);
        let node_position =
            patchright_dom_position_for_backend_node_id(&described_document, backend_node_id)
                .unwrap_or_default();
        assert!(!node_position.is_empty());
        current_ordered.push((
            backend_node_id,
            node_position,
            patchright_element_id_attr(&described),
        ));
    }

    current_ordered.sort_by(|left, right| {
        patchright_position_sort_key(&left.1).cmp(&patchright_position_sort_key(&right.1))
    });

    let mut seen_backend_node_ids = std::collections::HashSet::new();
    current_ordered
        .retain(|(backend_node_id, _, _)| seen_backend_node_ids.insert(*backend_node_id));

    let deduped_ids = current_ordered
        .iter()
        .map(|(_, _, id)| id.clone())
        .collect::<Vec<_>>();
    assert_eq!(deduped_ids, vec!["before", "host", "a", "b", "after"]);

    let mut anded_backend_node_ids = std::collections::HashSet::new();
    for (id, array_object_id) in [
        (16_u64, &array_object_ids[3]),
        (17_u64, &array_object_ids[4]),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": array_object_id,
                "arguments": [
                    { "value": 0 },
                    { "objectId": array_object_id }
                ],
                "functionDeclaration": "function(i, elems) { return elems[i]; }"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, id)["result"]["result"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(!object_id.is_empty());

        ctx.process_async(json!({
            "id": id + 10,
            "method": "DOM.describeNode",
            "params": {
                "objectId": object_id,
                "depth": -1
            }
        }))
        .await;
        let described = take_response_by_id(&mut ctx, id + 10)["result"]["node"].clone();
        let backend_node_id = described["backendNodeId"].as_u64().unwrap_or(0);
        assert!(backend_node_id > 0);
        anded_backend_node_ids.insert(backend_node_id);
    }

    assert_eq!(anded_backend_node_ids.len(), 2);

    let intersected_ids = current_ordered
        .into_iter()
        .filter(|(backend_node_id, _, _)| anded_backend_node_ids.contains(backend_node_id))
        .map(|(_, _, id)| id)
        .collect::<Vec<_>>();
    assert_eq!(intersected_ids, vec!["a", "after"]);
}
