use crate::{
    NodeId,
    native::{DomHost, Node},
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::num::NonZeroU32;

use super::ax_projection::{AxIgnoredReason, AxTreeProjection};
use super::ax_properties::{ax_name, ax_properties, ax_value};
use super::ax_roles::{ax_role, ordered_list_item_index};
use super::{AccessibilityInput, AccessibilityNode, AccessibilityNodeId};

/// The semantic traversal requested by one synchronous AX command.
#[derive(Clone, Copy, Debug)]
pub enum AccessibilityRequest {
    Tree { max_depth: Option<i32> },
    Node,
    Children,
    Ancestors,
    Partial { fetch_relatives: bool },
}

pub fn accessibility_payloads_for_document(
    document: &DomHost,
    input: &mut AccessibilityInput<'_>,
    node_id: NodeId,
    request: AccessibilityRequest,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<Vec<Value>> {
    build_nodes(document, input, node_id, request, backend_node_id_for_node)
}

pub fn accessibility_nodes_for_document(
    document: &DomHost,
    input: &mut AccessibilityInput<'_>,
    node_id: NodeId,
    request: AccessibilityRequest,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<Vec<AccessibilityNode>> {
    build_nodes(document, input, node_id, request, backend_node_id_for_node)
}

fn build_nodes<T: From<AccessibilityNode>>(
    document: &DomHost,
    input: &mut AccessibilityInput<'_>,
    node_id: NodeId,
    request: AccessibilityRequest,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<Vec<T>> {
    // A ShadowRoot contributes children but has no AX object of its own.
    let node_id = if matches!(request, AccessibilityRequest::Tree { .. }) {
        document.shadow_root_host(node_id).unwrap_or(node_id)
    } else {
        node_id
    };
    let mut projection = AxTreeProjection::new(document, node_id, input)?;
    match request {
        AccessibilityRequest::Tree { max_depth } => tree_payloads(
            &mut projection,
            node_id,
            max_depth,
            backend_node_id_for_node,
        ),
        AccessibilityRequest::Node => Some(vec![
            node_payload(&mut projection, node_id, backend_node_id_for_node)?.into(),
        ]),
        AccessibilityRequest::Children => {
            let mut nodes = Vec::new();
            if projection.is_rendered(node_id).ok()? {
                add_ax_children(
                    &mut projection,
                    node_id,
                    &mut nodes,
                    backend_node_id_for_node,
                )?;
            }
            Some(nodes)
        }
        AccessibilityRequest::Ancestors => {
            if projection.node(node_id).ok()?.is_none() {
                return Some(vec![
                    ax_dom_node_without_object_payload(node_id, backend_node_id_for_node)?.into(),
                ]);
            }
            let mut nodes = Vec::new();
            add_ax_ancestors(
                &mut projection,
                Some(node_id),
                &mut nodes,
                backend_node_id_for_node,
            )?;
            Some(nodes)
        }
        AccessibilityRequest::Partial { fetch_relatives } => {
            let projected = projection.node(node_id).ok()?;
            let mut nodes =
                vec![node_payload(&mut projection, node_id, backend_node_id_for_node)?.into()];
            if fetch_relatives {
                if projected.is_some_and(|node| node.ignored_reason.is_none()) {
                    add_ax_children(
                        &mut projection,
                        node_id,
                        &mut nodes,
                        backend_node_id_for_node,
                    )?;
                }
                let parent = if let Some(projected) = projected {
                    projected.parent
                } else {
                    let mut parent = document.composed_parent(node_id);
                    while let Some(id) = parent {
                        if projection.node(id).ok()?.is_some() {
                            break;
                        }
                        parent = document.composed_parent(id);
                    }
                    parent
                };
                add_ax_ancestors(
                    &mut projection,
                    parent,
                    &mut nodes,
                    backend_node_id_for_node,
                )?;
            }
            Some(nodes)
        }
    }
}

fn tree_payloads<T: From<AccessibilityNode>>(
    projection: &mut AxTreeProjection<'_, '_>,
    root: NodeId,
    max_depth: Option<i32>,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<Vec<T>> {
    if !projection.is_rendered(root).ok()? {
        return Some(Vec::new());
    }
    let mut nodes = Vec::new();
    push_ax_node_payload(projection, root, &mut nodes, backend_node_id_for_node)?;

    // Match Blink's WalkAXNodesToDepth: ignored nodes remain observable, but
    // only unignored nodes consume depth.
    let max_depth = max_depth.unwrap_or(-1);
    let mut pending = VecDeque::from([(root, 1)]);
    while let Some((node_id, depth)) = pending.pop_front() {
        add_ax_children(projection, node_id, &mut nodes, backend_node_id_for_node)?;
        if max_depth < 0 || depth < max_depth {
            pending.extend(
                projection
                    .unignored_children(node_id)
                    .ok()?
                    .into_iter()
                    .map(|child_id| (child_id, depth + 1)),
            );
        }
    }
    Some(nodes)
}

fn node_payload(
    projection: &mut AxTreeProjection<'_, '_>,
    node_id: NodeId,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<AccessibilityNode> {
    if projection.node(node_id).ok()?.is_none() {
        ax_dom_node_without_object_payload(node_id, backend_node_id_for_node)
    } else {
        ax_node_payload(projection, node_id, backend_node_id_for_node)
    }
}

fn add_ax_ancestors<T: From<AccessibilityNode>>(
    projection: &mut AxTreeProjection<'_, '_>,
    mut current: Option<NodeId>,
    out: &mut Vec<T>,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<()> {
    while let Some(node_id) = current {
        let projected = projection.node(node_id).ok()??;
        out.push(ax_node_payload(projection, node_id, backend_node_id_for_node)?.into());
        current = projected.parent;
    }
    Some(())
}

fn ax_dom_node_without_object_payload(
    node_id: NodeId,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<AccessibilityNode> {
    let backend_node_id = NonZeroU32::new(backend_node_id_for_node(node_id)?)?;
    let mut node = AccessibilityNode::new(AccessibilityNodeId::Dom(backend_node_id), "none");
    node.ignored = true;
    node.set_rare_data(
        Some(json!([{"name": "notRendered", "value": {"type": "boolean", "value": true}}])),
        None,
        None,
    );
    Some(node)
}

fn add_ax_children<T: From<AccessibilityNode>>(
    projection: &mut AxTreeProjection<'_, '_>,
    node_id: NodeId,
    out: &mut Vec<T>,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<()> {
    let mut reachable = projection.children(node_id).ok()?;
    reachable.reverse();
    while let Some(child_id) = reachable.pop() {
        let child = projection.node(child_id).ok()??;
        if child.ignored_reason == Some(AxIgnoredReason::NotRendered) {
            continue;
        }
        push_ax_node_payload(projection, child_id, out, backend_node_id_for_node)?;
        if child.ignored_reason.is_some() {
            reachable.extend(projection.children(child_id).ok()?.into_iter().rev());
        }
    }
    Some(())
}

fn push_ax_node_payload<T: From<AccessibilityNode>>(
    projection: &mut AxTreeProjection<'_, '_>,
    node_id: NodeId,
    out: &mut Vec<T>,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<()> {
    let projected = projection.node(node_id).ok()??;
    out.push(ax_node_payload(projection, node_id, backend_node_id_for_node)?.into());
    if projected.ignored_reason.is_none()
        && let Some(marker) = ax_list_marker_payload(
            projection.document,
            node_id,
            projection.document.node(node_id)?,
            backend_node_id_for_node,
        )
    {
        out.push(marker.into());
    }
    Some(())
}

fn ax_node_payload(
    projection: &mut AxTreeProjection<'_, '_>,
    node_id: NodeId,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<AccessibilityNode> {
    let document = projection.document;
    let node = document.node(node_id)?;
    let projected = projection.node(node_id).ok()??;
    let backend_node_id = NonZeroU32::new(backend_node_id_for_node(node_id)?)?;
    let ignored = projected.ignored_reason.is_some();
    let ignored_reasons = projected.ignored_reason.map(|reason| {
        ax_ignored_reasons_payload(document, node_id, reason, backend_node_id_for_node)
    });
    let name = if ignored {
        String::new()
    } else {
        ax_name(document, projection, node).ok()?
    };
    let value = if ignored {
        None
    } else {
        ax_value(document, node_id, node)
    };
    let properties = if ignored {
        Vec::new()
    } else {
        ax_properties(document, node_id, node)
    };
    let parent_id = match projected.parent {
        Some(parent) => Some(AccessibilityNodeId::Dom(NonZeroU32::new(
            backend_node_id_for_node(parent)?,
        )?)),
        None => None,
    };
    let children = projection.children(node_id).ok()?;
    let child_ids = if children.is_empty() {
        None
    } else {
        let mut child_ids = Vec::with_capacity(children.len());
        for child_id in children {
            if projection.is_rendered(child_id).ok()? {
                child_ids.push(AccessibilityNodeId::Dom(NonZeroU32::new(
                    backend_node_id_for_node(child_id)?,
                )?));
            }
        }
        Some(child_ids)
    };
    let mut payload = AccessibilityNode::new(
        AccessibilityNodeId::Dom(backend_node_id),
        if ignored { "none" } else { ax_role(node) },
    );
    payload.ignored = ignored;
    payload.name = (!name.is_empty()).then_some(name);
    payload.parent_id = parent_id;
    payload.child_ids = child_ids;
    payload.set_rare_data(ignored_reasons, value, Some(properties));
    Some(payload)
}

fn ax_ignored_reasons_payload(
    document: &DomHost,
    node_id: NodeId,
    reason: AxIgnoredReason,
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Value {
    match reason {
        AxIgnoredReason::Uninteresting
        | AxIgnoredReason::NotRendered
        | AxIgnoredReason::InertFrame
        | AxIgnoredReason::NotVisible => json!([{
            "name": match reason {
                AxIgnoredReason::NotRendered => "notRendered",
                AxIgnoredReason::InertFrame => "inertSubtree",
                AxIgnoredReason::NotVisible => "notVisible",
                _ => "uninteresting",
            },
            "value": {
                "type": "boolean",
                "value": true,
            }
        }]),
        AxIgnoredReason::AriaHiddenSubtree { root }
        | AxIgnoredReason::InertSubtree { root }
        | AxIgnoredReason::ActiveModalDialog { root } => {
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
                "name": match reason {
                    AxIgnoredReason::InertSubtree { .. } if root == node_id => "inertElement",
                    AxIgnoredReason::InertSubtree { .. } => "inertSubtree",
                    AxIgnoredReason::ActiveModalDialog { .. } => "activeModalDialog",
                    _ => "ariaHiddenSubtree",
                },
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
    backend_node_id_for_node: &mut dyn FnMut(NodeId) -> Option<u32>,
) -> Option<AccessibilityNode> {
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

    let parent_backend_node_id = NonZeroU32::new(backend_node_id_for_node(node_id)?)?;
    let mut payload = AccessibilityNode::new(
        AccessibilityNodeId::ListMarker(parent_backend_node_id),
        "ListMarker",
    );
    payload.name = Some(marker_text);
    payload.parent_id = Some(AccessibilityNodeId::Dom(parent_backend_node_id));
    payload.child_ids = Some(Vec::new());
    Some(payload)
}
