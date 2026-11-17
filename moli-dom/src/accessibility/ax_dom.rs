use crate::{
    NodeId,
    forms::{InputType, parse_html_integer_prefix},
    native::{DomHost, Element, NativeDom, Node},
};

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

/// Native href capability shared by role computation and URL publication.
/// Keep this aligned with the HTML/SVG links supported by activation.
pub(super) fn ax_link_href(element: &Element) -> Option<&str> {
    if element.is_html_element("a")
        || element.is_html_element("area")
        || element.is_html_element("link")
    {
        element.attribute_ns("", "href")
    } else if element.is_svg_element("a") {
        element
            .attribute_ns("", "href")
            .or_else(|| element.attribute_ns("http://www.w3.org/1999/xlink", "href"))
    } else {
        None
    }
}

/// DOM focus support for presentation-role conflicts. Hidden and inert nodes
/// are handled by the surrounding AX projection, without requesting layout.
pub(super) fn ax_supports_focus(document: &NativeDom, node: &Node) -> bool {
    let Some(element) = node.as_element() else {
        return false;
    };
    if element.is_html_input() && element.input_type() == InputType::Hidden {
        return false;
    }
    if element.namespace() == "http://www.w3.org/1999/xhtml"
        && matches!(
            element.local_name(),
            "button" | "input" | "select" | "textarea"
        )
        && ax_control_is_disabled(document, node.id(), element)
    {
        return false;
    }
    if element
        .attribute_ns("", "tabindex")
        .and_then(parse_html_integer_prefix)
        .is_some()
    {
        return true;
    }
    if element.is_html_element("a")
        || element.is_html_element("area")
        || element.is_svg_element("a")
    {
        return ax_link_href(element).is_some() || ax_is_editing_host(document, node);
    }
    if element.namespace() == "http://www.w3.org/1999/xhtml"
        && matches!(
            element.local_name(),
            "button" | "input" | "select" | "textarea"
        )
    {
        return true;
    }
    if element.is_html_element("summary")
        && let Some(parent) = document.parent_node(node.id())
        && document
            .node(parent)
            .and_then(Node::as_element)
            .is_some_and(|parent| parent.is_html_element("details"))
    {
        return document.child_ids(parent).find(|child| {
            document
                .node(*child)
                .and_then(Node::as_element)
                .is_some_and(|child| child.is_html_element("summary"))
        }) == Some(node.id());
    }
    ax_is_editing_host(document, node)
}

fn ax_is_editing_host(document: &NativeDom, node: &Node) -> bool {
    fn editable(element: &Element) -> Option<bool> {
        let value = element.attribute_ns("", "contenteditable")?;
        if value.is_empty()
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("plaintext-only")
        {
            Some(true)
        } else if value.eq_ignore_ascii_case("false") {
            Some(false)
        } else {
            None
        }
    }
    if node.as_element().and_then(editable) != Some(true) {
        return false;
    }
    let mut ancestor = document.parent_node(node.id());
    while let Some(id) = ancestor {
        let Some(parent) = document.node(id) else {
            break;
        };
        if let Some(is_editable) = parent.as_element().and_then(editable) {
            return !is_editable;
        }
        ancestor = parent.parent_node_id();
    }
    true
}

pub(super) fn ax_control_is_disabled(
    document: &NativeDom,
    node_id: NodeId,
    element: &Element,
) -> bool {
    if element.has_attribute("disabled") {
        return true;
    }
    if !matches!(
        element.local_name(),
        "button" | "input" | "select" | "textarea"
    ) {
        return false;
    }

    let mut ancestor = document.parent_node(node_id);
    while let Some(ancestor_id) = ancestor {
        let Some(ancestor_node) = document.node(ancestor_id) else {
            break;
        };
        if ancestor_node.as_element().is_some_and(|ancestor| {
            ancestor.is_html_fieldset() && ancestor.has_attribute("disabled")
        }) && !ax_is_inside_first_legend(document, node_id, ancestor_id)
        {
            return true;
        }
        ancestor = ancestor_node.parent_node_id();
    }
    false
}

fn ax_is_inside_first_legend(document: &NativeDom, node_id: NodeId, fieldset_id: NodeId) -> bool {
    let Some(first_legend) = document.child_ids(fieldset_id).find(|child_id| {
        document
            .node(*child_id)
            .and_then(Node::as_element)
            .is_some_and(|element| element.is_html_element("legend"))
    }) else {
        return false;
    };

    let mut current = Some(node_id);
    while let Some(current_id) = current {
        if current_id == first_legend {
            return true;
        }
        if current_id == fieldset_id {
            return false;
        }
        current = document.parent_node(current_id);
    }
    false
}
