use crate::{
    NodeId,
    native::{DomHost, Node},
};
use serde_json::{Value, json};
use std::collections::VecDeque;

use super::ax_projection::{AxIgnoredReason, AxTreeProjection};
use super::ax_properties::{ax_name, ax_properties, ax_value};
use super::ax_roles::{ax_role, cdp_node_id, ordered_list_item_index};

// ---------------------------------------------------------------------------
// Public (pub(crate)) tree-building entry points
// ---------------------------------------------------------------------------

pub fn accessibility_tree_payloads_for_document(
    document: &DomHost,
    root: NodeId,
    max_depth: Option<i32>,
) -> Vec<Value> {
    let mut backend_node_id_for_node = |node_id| Some(cdp_node_id(node_id));
    accessibility_tree_payloads_for_document_with_backend_node_ids(
        document,
        root,
        max_depth,
        &mut backend_node_id_for_node,
    )
    .unwrap_or_default()
}

pub fn accessibility_tree_payloads_for_document_with_backend_node_ids(
    document: &DomHost,
    root: NodeId,
    max_depth: Option<i32>,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> Option<Vec<Value>> {
    // Blink's queryAXTree starts at the host when given a ShadowRoot, which
    // contributes descendants but has no AX node of its own.
    let root = document.shadow_root_host(root).unwrap_or(root);
    let projection = AxTreeProjection::build_for_node(document, root);
    if !projection.contains(root) {
        return Some(Vec::new());
    }

    let mut nodes = Vec::new();
    if !push_ax_node_payload(
        document,
        &projection,
        root,
        &mut nodes,
        backend_node_id_for_node,
    ) {
        return None;
    }

    // Match Blink's WalkAXNodesToDepth: ignored nodes remain observable, but
    // only unignored nodes consume depth. Each queued node exposes its direct
    // children plus any ignored chains between it and the next semantic layer.
    let max_depth = max_depth.unwrap_or(-1);
    let mut pending = VecDeque::from([(root, 1)]);
    while let Some((node_id, depth)) = pending.pop_front() {
        add_ax_children(
            document,
            &projection,
            node_id,
            &mut nodes,
            backend_node_id_for_node,
        );
        if max_depth < 0 || depth < max_depth {
            pending.extend(
                projection
                    .unignored_children(node_id)
                    .into_iter()
                    .map(|child_id| (child_id, depth + 1)),
            );
        }
    }
    Some(nodes)
}

pub fn accessibility_node_payload_for_document(
    document: &DomHost,
    node_id: NodeId,
) -> Option<Value> {
    let mut backend_node_id_for_node = |node_id| Some(cdp_node_id(node_id));
    accessibility_node_payload_for_document_with_backend_node_ids(
        document,
        node_id,
        &mut backend_node_id_for_node,
    )
}

pub fn accessibility_node_payload_for_document_with_backend_node_ids(
    document: &DomHost,
    node_id: NodeId,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> Option<Value> {
    let projection = AxTreeProjection::build_for_node(document, node_id);
    let projected = projection.node(node_id)?;
    let node = document.node(node_id)?;
    ax_node_payload(document, node_id, node, projected, backend_node_id_for_node)
}

pub fn accessibility_child_node_payloads_for_document(
    document: &DomHost,
    node_id: NodeId,
) -> Vec<Value> {
    let mut backend_node_id_for_node = |node_id| Some(cdp_node_id(node_id));
    accessibility_child_node_payloads_for_document_with_backend_node_ids(
        document,
        node_id,
        &mut backend_node_id_for_node,
    )
    .unwrap_or_default()
}

pub fn accessibility_child_node_payloads_for_document_with_backend_node_ids(
    document: &DomHost,
    node_id: NodeId,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> Option<Vec<Value>> {
    let projection = AxTreeProjection::build_for_node(document, node_id);
    if !projection.contains(node_id) {
        return Some(Vec::new());
    }
    let mut nodes = Vec::new();
    add_ax_children(
        document,
        &projection,
        node_id,
        &mut nodes,
        backend_node_id_for_node,
    );
    Some(nodes)
}

pub fn accessibility_node_and_ancestor_payloads_for_document(
    document: &DomHost,
    node_id: NodeId,
) -> Vec<Value> {
    let mut backend_node_id_for_node = |node_id| Some(cdp_node_id(node_id));
    accessibility_node_and_ancestor_payloads_for_document_with_backend_node_ids(
        document,
        node_id,
        &mut backend_node_id_for_node,
    )
    .unwrap_or_default()
}

pub fn accessibility_node_and_ancestor_payloads_for_document_with_backend_node_ids(
    document: &DomHost,
    node_id: NodeId,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> Option<Vec<Value>> {
    let projection = AxTreeProjection::build_for_node(document, node_id);
    let mut chain = Vec::new();
    let mut current = Some(node_id);
    while let Some(current_id) = current {
        let Some(node) = document.node(current_id) else {
            break;
        };
        let Some(projected) = projection.node(current_id) else {
            break;
        };
        chain.push(ax_node_payload(
            document,
            current_id,
            node,
            projected,
            backend_node_id_for_node,
        )?);
        current = projected.parent;
    }
    Some(chain)
}

pub fn accessibility_partial_tree_payloads_for_document(
    document: &DomHost,
    node_id: NodeId,
    fetch_relatives: bool,
) -> Option<Vec<Value>> {
    let mut backend_node_id_for_node = |node_id| Some(cdp_node_id(node_id));
    accessibility_partial_tree_payloads_for_document_with_backend_node_ids(
        document,
        node_id,
        fetch_relatives,
        &mut backend_node_id_for_node,
    )
}

pub fn accessibility_partial_tree_payloads_for_document_with_backend_node_ids(
    document: &DomHost,
    node_id: NodeId,
    fetch_relatives: bool,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> Option<Vec<Value>> {
    let projection = AxTreeProjection::build_for_node(document, node_id);
    let node = document.node(node_id)?;
    let projected = projection.node(node_id)?;
    let mut nodes = vec![ax_node_payload(
        document,
        node_id,
        node,
        projected,
        backend_node_id_for_node,
    )?];

    if fetch_relatives {
        if projected.ignored_reason.is_none() {
            add_ax_children(
                document,
                &projection,
                node_id,
                &mut nodes,
                backend_node_id_for_node,
            );
        }

        let mut current = projected.parent;
        while let Some(current_id) = current {
            let Some(current_node) = document.node(current_id) else {
                break;
            };
            let Some(current_projected) = projection.node(current_id) else {
                break;
            };
            nodes.push(ax_node_payload(
                document,
                current_id,
                current_node,
                current_projected,
                backend_node_id_for_node,
            )?);
            current = current_projected.parent;
        }
    }

    Some(nodes)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn add_ax_children(
    document: &DomHost,
    projection: &AxTreeProjection,
    node_id: NodeId,
    out: &mut Vec<Value>,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) {
    let Some(projected) = projection.node(node_id) else {
        return;
    };

    let mut reachable = projected.children.iter().rev().copied().collect::<Vec<_>>();
    while let Some(child_id) = reachable.pop() {
        let Some(child) = projection.node(child_id) else {
            continue;
        };
        push_ax_node_payload(
            document,
            projection,
            child_id,
            out,
            backend_node_id_for_node,
        );
        if child.ignored_reason.is_some() {
            reachable.extend(child.children.iter().rev().copied());
        }
    }
}

fn push_ax_node_payload(
    document: &DomHost,
    projection: &AxTreeProjection,
    node_id: NodeId,
    out: &mut Vec<Value>,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> bool {
    let Some(node) = document.node(node_id) else {
        return false;
    };
    let Some(projected) = projection.node(node_id) else {
        return false;
    };
    let Some(payload) =
        ax_node_payload(document, node_id, node, projected, backend_node_id_for_node)
    else {
        return false;
    };
    out.push(payload);
    if projected.ignored_reason.is_none()
        && let Some(marker) =
            ax_list_marker_payload(document, node_id, node, backend_node_id_for_node)
    {
        out.push(marker);
    }
    true
}

fn ax_node_payload(
    document: &DomHost,
    node_id: NodeId,
    node: &Node,
    projected: &super::ax_projection::AxProjectedNode,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> Option<Value> {
    let backend_node_id = backend_node_id_for_node(node_id)?;
    let mut payload = serde_json::Map::new();
    payload.insert("nodeId".to_owned(), json!(format!("AX-{backend_node_id}")));
    payload.insert("backendDOMNodeId".to_owned(), json!(backend_node_id));
    let ignored = projected.ignored_reason.is_some();
    payload.insert("ignored".to_owned(), json!(ignored));
    if let Some(reason) = projected.ignored_reason {
        payload.insert(
            "ignoredReasons".to_owned(),
            ax_ignored_reasons_payload(document, reason, backend_node_id_for_node),
        );
    }
    payload.insert(
        "role".to_owned(),
        json!({
            "type": "role",
            "value": if ignored { "none" } else { ax_role(node) },
        }),
    );

    if !ignored {
        let name = ax_name(document, node);
        if !name.is_empty() {
            payload.insert(
                "name".to_owned(),
                json!({
                    "type": "computedString",
                    "value": name,
                }),
            );
        }

        if let Some(value) = ax_value(document, node_id, node) {
            payload.insert("value".to_owned(), value);
        }
    }

    payload.insert(
        "properties".to_owned(),
        Value::Array(if ignored {
            Vec::new()
        } else {
            ax_properties(document, node_id, node)
        }),
    );

    if let Some(parent_id) = projected.parent
        && let Some(parent_backend_node_id) = backend_node_id_for_node(parent_id)
    {
        payload.insert(
            "parentId".to_owned(),
            json!(format!("AX-{parent_backend_node_id}")),
        );
    }

    if !projected.children.is_empty() {
        let child_ids = projected
            .children
            .iter()
            .filter_map(|child_id| {
                backend_node_id_for_node(*child_id)
                    .map(|backend_node_id| format!("AX-{backend_node_id}"))
            })
            .collect::<Vec<_>>();
        payload.insert("childIds".to_owned(), json!(child_ids));
    }

    Some(Value::Object(payload))
}

fn ax_ignored_reasons_payload(
    document: &DomHost,
    reason: AxIgnoredReason,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> Value {
    match reason {
        AxIgnoredReason::Uninteresting => json!([{
            "name": "uninteresting",
            "value": {
                "type": "boolean",
                "value": true,
            }
        }]),
        AxIgnoredReason::AriaHiddenSubtree { root } => {
            let mut related_node = serde_json::Map::new();
            if let Some(backend_node_id) = backend_node_id_for_node(root) {
                related_node.insert("backendDOMNodeId".to_owned(), json!(backend_node_id));
            }
            if let Some(idref) = document
                .node(root)
                .and_then(Node::as_element)
                .and_then(|element| element.id())
            {
                related_node.insert("idref".to_owned(), json!(idref));
            }
            json!([{
                "name": "ariaHiddenSubtree",
                "value": {
                    "type": "idref",
                    "relatedNodes": [Value::Object(related_node)],
                }
            }])
        }
    }
}

fn ax_list_marker_payload(
    document: &DomHost,
    node_id: NodeId,
    node: &Node,
    backend_node_id_for_node: &mut impl FnMut(NodeId) -> Option<u32>,
) -> Option<Value> {
    let element = node.as_element()?;
    if !element.is_html_element("li") {
        return None;
    }

    let parent_id = node.parent_node_id()?;
    let parent = document.node(parent_id)?;
    let parent_element = parent.as_element()?;
    let marker_text = match parent_element.local_name() {
        "ul" | "menu" => "\u{2022} ".to_owned(),
        "ol" => format!(
            "{}. ",
            ordered_list_item_index(document, parent_id, node_id)?
        ),
        _ => return None,
    };

    let parent_backend_node_id = backend_node_id_for_node(node_id)?;
    Some(json!({
        "nodeId": format!("AX-LM-{parent_backend_node_id}"),
        "ignored": false,
        "role": {
            "type": "role",
            "value": "ListMarker",
        },
        "name": {
            "type": "computedString",
            "value": marker_text,
        },
        "properties": [],
        "parentId": format!("AX-{parent_backend_node_id}"),
        "childIds": [],
    }))
}
