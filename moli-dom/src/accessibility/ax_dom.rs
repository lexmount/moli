use crate::{NodeData, NodeId, native::DomHost};

use super::ax_projection::{AxTreeProjection, AxUnavailable};

pub(super) const MAX_AX_NAME_VISITED_OBJECTS: usize = 100;

pub(super) fn ax_child_ids(
    document: &DomHost,
    node_id: NodeId,
) -> impl Iterator<Item = NodeId> + '_ {
    let composed = composed_child_ids(document, node_id);
    let ordinary = composed.is_none().then(|| document.child_ids(node_id));
    composed
        .into_iter()
        .flatten()
        .chain(ordinary.into_iter().flatten())
}

pub(super) fn ax_child_ids_reversed(
    document: &DomHost,
    node_id: NodeId,
) -> impl Iterator<Item = NodeId> + '_ {
    let composed = composed_child_ids(document, node_id);
    let ordinary = composed
        .is_none()
        .then(|| document.child_ids_reversed(node_id));
    composed
        .into_iter()
        .flat_map(|children| children.into_iter().rev())
        .chain(ordinary.into_iter().flatten())
}

fn composed_child_ids(document: &DomHost, node_id: NodeId) -> Option<Vec<NodeId>> {
    // Shadow trees replace the host's light children; slots reinsert assigned
    // nodes in their rendered order, using fallback children when unassigned.
    if let Some(root) = document.shadow_root_handle(node_id) {
        return Some(vec![root]);
    }
    if document
        .element(node_id)
        .is_some_and(|element| element.is_html_element("slot"))
        && document.containing_shadow_root(node_id).is_some()
    {
        let assigned = document.assigned_nodes_for_slot_with_options(node_id, false);
        if !assigned.is_empty() {
            return Some(assigned);
        }
    }
    None
}

pub(super) fn ax_content_text(
    document: &DomHost,
    projection: &mut AxTreeProjection<'_, '_>,
    node_id: NodeId,
    include_hidden: bool,
) -> Result<String, AxUnavailable> {
    if projection.state(node_id)?.hides_contents && !include_hidden {
        return Ok(String::new());
    }
    let mut text = String::new();
    let mut pending = ax_child_ids_reversed(document, node_id)
        .map(Some)
        .collect::<Vec<_>>();
    let mut remaining = MAX_AX_NAME_VISITED_OBJECTS;
    while let Some(entry) = pending.pop() {
        let Some(node_id) = entry else {
            text.push(' ');
            continue;
        };
        if remaining == 0 {
            break;
        }
        remaining -= 1;
        let Some(node) = document.node(node_id) else {
            continue;
        };
        let state = projection.state(node_id)?;
        if state.inert_reason.is_some()
            || (!include_hidden && (state.not_rendered || state.aria_hidden_root.is_some()))
        {
            continue;
        }
        // Unrendered ARIA references have no shared inline formatting context.
        if state.block_level || (include_hidden && state.not_rendered) {
            text.push(' ');
            pending.push(None);
        }
        if state.hides_contents && !include_hidden {
            continue;
        }
        match node.kind() {
            NodeData::Text(value) if include_hidden || state.visibility_visible => {
                text.push_str(value.data())
            }
            NodeData::CDataSection(value) if include_hidden || state.visibility_visible => {
                text.push_str(value.data())
            }
            _ => pending.extend(ax_child_ids_reversed(document, node_id).map(Some)),
        }
    }
    Ok(text)
}
