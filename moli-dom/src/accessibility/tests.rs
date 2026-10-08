use serde_json::{Value, json};

use super::*;
use crate::{
    NodeId,
    native::{DomHost, NativeDom},
};

fn tree(document: &DomHost) -> Vec<Value> {
    accessibility_tree_payloads_for_document(document, document.document_node_id(), None)
}

fn document_with_host() -> (DomHost, NodeId) {
    let mut document = DomHost::from_dom(NativeDom::new_html(
        url::Url::parse("https://example.test/").expect("valid document URL"),
    ));
    let host = append_element(&mut document, NodeId::new(0), "div");
    (document, host)
}

fn append_element(document: &mut DomHost, parent: NodeId, tag: &str) -> NodeId {
    let element = document.create_element(tag);
    assert!(document.append_child(parent, element));
    element
}

fn append_text(document: &mut DomHost, parent: NodeId, text: &str) {
    let text = document.create_text_node(text);
    assert!(document.append_child(parent, text));
}

fn node_by_id(nodes: &[Value], node_id: NodeId) -> &Value {
    let backend_id = super::ax_roles::cdp_node_id(node_id);
    nodes
        .iter()
        .find(|node| node["backendDOMNodeId"] == backend_id)
        .unwrap_or_else(|| panic!("missing AX node for {node_id:?}: {nodes:?}"))
}

fn button_names(nodes: &[Value]) -> Vec<&str> {
    nodes
        .iter()
        .filter(|node| node["role"]["value"] == "button")
        .map(|node| node["name"]["value"].as_str().expect("button name"))
        .collect()
}

#[test]
fn ax_tree_queries_shadow_roots_from_hosts() {
    for mode in ["open", "closed"] {
        let (mut document, host) = document_with_host();
        let root = document
            .attach_shadow_root(host, mode)
            .expect("shadow root");
        let button = append_element(&mut document, root, "button");
        append_text(&mut document, button, "Shadow action");
        let document_id = document.document_node_id();
        let outside = append_element(&mut document, document_id, "button");
        append_text(&mut document, outside, "Outside action");

        let nodes = accessibility_tree_payloads_for_document(&document, root, None);
        assert_eq!(
            nodes[0]["backendDOMNodeId"],
            super::ax_roles::cdp_node_id(host)
        );
        assert_eq!(button_names(&nodes), ["Shadow action"]);
        assert_eq!(
            node_by_id(&nodes, button)["nodeId"],
            node_by_id(&tree(&document), button)["nodeId"]
        );
        assert!(!nodes.iter().any(|node| {
            node["backendDOMNodeId"] == super::ax_roles::cdp_node_id(root)
                || node["backendDOMNodeId"] == super::ax_roles::cdp_node_id(outside)
        }));
    }
}

#[test]
fn ax_tree_names_shadow_hosts_from_composed_contents() {
    for mode in ["open", "closed"] {
        let (mut document, host) = document_with_host();
        assert!(document.set_attribute(host, "role", "button"));
        append_text(&mut document, host, "host name");
        let root = document
            .attach_shadow_root(host, mode)
            .expect("shadow root");
        append_text(&mut document, root, "Shadow ");
        append_element(&mut document, root, "slot");

        assert_eq!(
            node_by_id(&tree(&document), host)["name"]["value"],
            "Shadow host name"
        );
    }
}

#[test]
fn ax_tree_includes_nested_shadow_controls_and_host_ancestors() {
    for mode in ["open", "closed"] {
        let (mut document, host) = document_with_host();
        let root = document
            .attach_shadow_root(host, mode)
            .expect("shadow root");
        let nested_host = append_element(&mut document, root, "div");
        let nested_root = document
            .attach_shadow_root(nested_host, mode)
            .expect("nested shadow root");
        let button = append_element(&mut document, nested_root, "button");
        append_text(&mut document, button, "Shadow action");

        let nodes = tree(&document);
        let button_node = node_by_id(&nodes, button);
        assert_eq!(button_node["role"]["value"], "button");
        assert_eq!(button_node["name"]["value"], "Shadow action");
        assert_eq!(button_node["ignored"], false);
        assert_eq!(
            button_node["parentId"],
            node_by_id(&nodes, nested_host)["nodeId"]
        );
        assert_eq!(
            node_by_id(&nodes, nested_host)["parentId"],
            node_by_id(&nodes, host)["nodeId"]
        );

        let ancestors = accessibility_node_and_ancestor_payloads_for_document(&document, button);
        let backend_ids = ancestors
            .iter()
            .map(|node| node["backendDOMNodeId"].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            backend_ids,
            [button, nested_host, host, document.document_node_id()]
                .map(|id| json!(super::ax_roles::cdp_node_id(id)))
        );
    }
}

#[test]
fn ax_tree_uses_slot_order_and_replaces_fallback_after_assignment_changes() {
    let (mut document, host) = document_with_host();
    let first = append_element(&mut document, host, "button");
    assert!(document.set_attribute(first, "slot", "second"));
    append_text(&mut document, first, "First");
    let second = append_element(&mut document, host, "button");
    assert!(document.set_attribute(second, "slot", "first"));
    append_text(&mut document, second, "Second");
    let unassigned = append_element(&mut document, host, "button");
    append_text(&mut document, unassigned, "Unassigned");

    let root = document
        .attach_shadow_root(host, "open")
        .expect("shadow root");
    let first_slot = append_element(&mut document, root, "slot");
    assert!(document.set_attribute(first_slot, "name", "first"));
    let fallback = append_element(&mut document, first_slot, "button");
    append_text(&mut document, fallback, "Fallback");
    let second_slot = append_element(&mut document, root, "slot");
    assert!(document.set_attribute(second_slot, "name", "second"));

    let nodes = tree(&document);
    assert_eq!(button_names(&nodes), ["Second", "First"]);
    assert_eq!(
        node_by_id(&nodes, second)["parentId"],
        node_by_id(&nodes, first_slot)["nodeId"]
    );

    assert!(document.set_attribute(second, "slot", "missing"));
    let nodes = tree(&document);
    assert_eq!(button_names(&nodes), ["Fallback", "First"]);
}

#[test]
fn ax_tree_names_shadow_controls_from_assigned_text_and_scoped_labels() {
    let (mut document, host) = document_with_host();
    let assigned = append_element(&mut document, host, "span");
    append_text(&mut document, assigned, "Assigned name");
    let document_id = document.document_node_id();
    let outside_label = append_element(&mut document, document_id, "span");
    assert!(document.set_attribute(outside_label, "id", "label"));
    append_text(&mut document, outside_label, "Outside label");

    let root = document
        .attach_shadow_root(host, "open")
        .expect("shadow root");
    let button = append_element(&mut document, root, "button");
    let slot = append_element(&mut document, button, "slot");
    append_text(&mut document, slot, "Fallback name");
    let label = append_element(&mut document, root, "span");
    assert!(document.set_attribute(label, "id", "label"));
    append_text(&mut document, label, "Shadow label");
    let labelled_button = append_element(&mut document, root, "button");
    assert!(document.set_attribute(labelled_button, "aria-labelledby", "label"));

    let nodes = tree(&document);
    assert_eq!(node_by_id(&nodes, button)["name"]["value"], "Assigned name");
    assert_eq!(
        node_by_id(&nodes, labelled_button)["name"]["value"],
        "Shadow label"
    );
}

#[test]
fn ax_tree_inherits_aria_hidden_from_shadow_host() {
    let (mut document, host) = document_with_host();
    assert!(document.set_attribute(host, "aria-hidden", "true"));
    let root = document
        .attach_shadow_root(host, "open")
        .expect("shadow root");
    let button = append_element(&mut document, root, "button");
    append_text(&mut document, button, "Hidden action");

    let nodes = tree(&document);
    let button_node = node_by_id(&nodes, button);
    assert_eq!(button_node["ignored"], true);
    assert_eq!(
        button_node["ignoredReasons"][0]["name"],
        "ariaHiddenSubtree"
    );
    assert_eq!(
        button_node["ignoredReasons"][0]["value"]["relatedNodes"][0]["backendDOMNodeId"],
        super::ax_roles::cdp_node_id(host)
    );
}
