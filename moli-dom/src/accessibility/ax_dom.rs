use crate::{NodeData, NodeId, native::DomHost};

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

pub(super) fn ax_content_text(document: &DomHost, node_id: NodeId) -> String {
    let mut text = String::new();
    let mut pending = ax_child_ids_reversed(document, node_id).collect::<Vec<_>>();
    while let Some(node_id) = pending.pop() {
        let Some(node) = document.node(node_id) else {
            continue;
        };
        match node.kind() {
            NodeData::Text(value) => text.push_str(value.data()),
            NodeData::CDataSection(value) => text.push_str(value.data()),
            _ => pending.extend(ax_child_ids_reversed(document, node_id)),
        }
    }
    text
}
