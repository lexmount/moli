use crate::{
    NodeData, NodeId,
    forms::InputType,
    native::{Element, NativeDom, Node},
};
use serde_json::{Value, json};
use std::collections::HashSet;
use url::Url;

use super::ax_roles::{ax_role, heading_level};

// Blink bounds text-alternative traversal with
// `kMaxDescendantsForTextAlternativeComputation`. Keep relation recursion
// within the same visited-object budget so hostile ARIA graphs cannot grow the
// native call stack without bound.
const MAX_AX_NAME_VISITED_OBJECTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxNameTraversal {
    Direct,
    AriaReference,
    NativeLabel,
}

impl AxNameTraversal {
    fn follows_aria_labelledby(self) -> bool {
        !matches!(self, Self::AriaReference)
    }

    fn includes_contents(self) -> bool {
        !matches!(self, Self::Direct)
    }
}

pub(super) fn ax_name(document: &NativeDom, node: &Node) -> String {
    match node.kind() {
        NodeData::Document(_) => ax_document_name(document),
        NodeData::Element(_) => ax_node_name(
            document,
            node.id(),
            AxNameTraversal::Direct,
            &mut HashSet::new(),
        ),
        NodeData::Text(text) => normalize_ax_whitespace(text.data()),
        NodeData::CDataSection(cdata) => normalize_ax_whitespace(cdata.data()),
        NodeData::Comment(_)
        | NodeData::ProcessingInstruction(_)
        | NodeData::DocumentType(_)
        | NodeData::DocumentFragment(_) => String::new(),
    }
}

/// Computes the subset of the accessible-name algorithm that is observable in
/// the CDP AX tree. The source order mirrors Blink's `TextAlternative`:
/// `aria-labelledby`, `aria-label`, native HTML alternatives, contents, then
/// tooltip-style fallbacks.
fn ax_node_name(
    document: &NativeDom,
    node_id: NodeId,
    traversal: AxNameTraversal,
    visited: &mut HashSet<NodeId>,
) -> String {
    if visited.len() >= MAX_AX_NAME_VISITED_OBJECTS || !visited.insert(node_id) {
        return String::new();
    }

    let name = match document.node(node_id) {
        Some(node) => match node.kind() {
            NodeData::Element(element) => {
                ax_element_name(document, node_id, node, element, traversal, visited)
            }
            NodeData::Text(text) => normalize_ax_whitespace(text.data()),
            NodeData::CDataSection(cdata) => normalize_ax_whitespace(cdata.data()),
            NodeData::Document(_) => ax_document_name(document),
            NodeData::Comment(_)
            | NodeData::ProcessingInstruction(_)
            | NodeData::DocumentType(_)
            | NodeData::DocumentFragment(_) => String::new(),
        },
        None => String::new(),
    };

    visited.remove(&node_id);
    name
}

fn ax_element_name(
    document: &NativeDom,
    node_id: NodeId,
    node: &Node,
    element: &Element,
    traversal: AxNameTraversal,
    visited: &mut HashSet<NodeId>,
) -> String {
    if traversal.follows_aria_labelledby()
        && let Some(labelled_by) = element
            .attribute("aria-labelledby")
            .or_else(|| element.attribute("aria-labeledby"))
    {
        let tree_scope = ax_tree_scope_root(document, node_id);
        let label_ids = labelled_by
            .split_whitespace()
            .filter_map(|id| ax_element_by_id(document, tree_scope, id))
            .collect::<Vec<_>>();
        if !label_ids.is_empty() {
            return label_ids
                .into_iter()
                .map(|label_id| {
                    ax_node_name(document, label_id, AxNameTraversal::AriaReference, visited)
                })
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
        }
    }

    if let Some(aria_label) = element.attribute("aria-label") {
        let aria_label = normalize_ax_whitespace(aria_label);
        if !aria_label.is_empty() {
            return aria_label;
        }
    }

    if let Some(native_name) = ax_native_element_name(document, node_id, element, visited) {
        return native_name;
    }

    // A node reached through aria-labelledby or a native <label> contributes
    // its whole subtree. Outside that recursive context, only roles whose name
    // comes from contents may consume descendant text; generic containers and
    // text controls expose that text through children or AXValue instead.
    let content =
        if traversal.includes_contents() || ax_name_comes_from_contents(document, node, element) {
            normalize_ax_whitespace(&node.text_content(document))
        } else {
            String::new()
        };
    if !content.is_empty() {
        return content;
    }

    element
        .attribute("title")
        .map(normalize_ax_whitespace)
        .filter(|title| !title.is_empty())
        .or_else(|| {
            ax_is_text_control(element)
                .then(|| {
                    element
                        .attribute("placeholder")
                        .map(normalize_ax_whitespace)
                })
                .flatten()
                .filter(|placeholder| !placeholder.is_empty())
        })
        .unwrap_or_default()
}

fn ax_native_element_name(
    document: &NativeDom,
    node_id: NodeId,
    element: &Element,
    visited: &mut HashSet<NodeId>,
) -> Option<String> {
    if ax_is_labelable(element) {
        let labels = ax_labels_for_control(document, node_id);
        if !labels.is_empty() {
            return Some(
                labels
                    .into_iter()
                    .map(|label_id| {
                        ax_node_name(document, label_id, AxNameTraversal::NativeLabel, visited)
                    })
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }

    match element.local_name() {
        "input" => match element.input_type() {
            InputType::Button => Some(normalize_ax_whitespace(&element.input_value())),
            InputType::Submit => Some(ax_input_button_name(element, "Submit")),
            InputType::Reset => Some(ax_input_button_name(element, "Reset")),
            InputType::Image => element
                .attribute("alt")
                .map(normalize_ax_whitespace)
                .or_else(|| {
                    let value = normalize_ax_whitespace(&element.input_value());
                    (!value.is_empty()).then_some(value)
                })
                .or_else(|| Some("Submit".to_owned())),
            _ => None,
        },
        "img" | "area" => element.attribute("alt").map(normalize_ax_whitespace),
        _ => None,
    }
}

fn ax_input_button_name(element: &Element, default_name: &str) -> String {
    let value = normalize_ax_whitespace(&element.input_value());
    if value.is_empty() {
        default_name.to_owned()
    } else {
        value
    }
}

fn ax_labels_for_control(document: &NativeDom, control_id: NodeId) -> Vec<NodeId> {
    let Some(control) = document.node(control_id).and_then(Node::as_element) else {
        return Vec::new();
    };
    if !ax_is_labelable(control) {
        return Vec::new();
    }

    let tree_scope = ax_tree_scope_root(document, control_id);
    document
        .elements_by_tag_name(tree_scope, "label", true)
        .into_iter()
        .filter(|label_id| ax_label_controls(document, *label_id, control_id, tree_scope))
        .collect()
}

fn ax_label_controls(
    document: &NativeDom,
    label_id: NodeId,
    control_id: NodeId,
    tree_scope: NodeId,
) -> bool {
    let Some(label) = document.node(label_id).and_then(Node::as_element) else {
        return false;
    };
    if !label.is_html_label() {
        return false;
    }

    if let Some(for_id) = label.attribute("for") {
        return ax_element_by_id(document, tree_scope, for_id) == Some(control_id)
            && document
                .node(control_id)
                .and_then(Node::as_element)
                .is_some_and(ax_is_labelable);
    }

    ax_first_labelable_descendant(document, label_id) == Some(control_id)
}

fn ax_first_labelable_descendant(document: &NativeDom, label_id: NodeId) -> Option<NodeId> {
    let mut stack = document.child_ids_reversed(label_id).collect::<Vec<_>>();
    while let Some(node_id) = stack.pop() {
        if document
            .node(node_id)
            .and_then(Node::as_element)
            .is_some_and(ax_is_labelable)
        {
            return Some(node_id);
        }
        stack.extend(document.child_ids_reversed(node_id));
    }
    None
}

fn ax_is_labelable(element: &Element) -> bool {
    if element.namespace() != "http://www.w3.org/1999/xhtml" {
        return false;
    }
    match element.local_name() {
        "button" | "meter" | "output" | "progress" | "select" | "textarea" => true,
        "input" => element.is_html_input() && element.input_type() != InputType::Hidden,
        _ => false,
    }
}

fn ax_is_text_control(element: &Element) -> bool {
    element.is_html_textarea()
        || (element.is_html_input()
            && matches!(
                element.input_type(),
                InputType::Email
                    | InputType::Number
                    | InputType::Password
                    | InputType::Search
                    | InputType::Tel
                    | InputType::Text
                    | InputType::Url
            ))
}

fn ax_name_comes_from_contents(document: &NativeDom, node: &Node, element: &Element) -> bool {
    if ax_role(node) == "row" {
        return ax_row_name_comes_from_contents(document, node);
    }

    matches!(
        ax_role(node),
        "button"
            | "cell"
            | "checkbox"
            | "columnheader"
            | "gridcell"
            | "heading"
            | "link"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "option"
            | "radio"
            | "rowheader"
            | "switch"
            | "tab"
            | "tooltip"
            | "treeitem"
    ) || matches!(element.local_name(), "caption" | "figcaption" | "legend")
}

fn ax_row_name_comes_from_contents(document: &NativeDom, node: &Node) -> bool {
    let mut ancestor = node.parent_node_id();
    while let Some(ancestor_id) = ancestor {
        let Some(ancestor_node) = document.node(ancestor_id) else {
            return false;
        };
        match ax_role(ancestor_node) {
            "grid" | "treegrid" => return true,
            "generic" | "none" | "group" | "rowgroup" => {
                ancestor = ancestor_node.parent_node_id();
            }
            _ => return false,
        }
    }
    false
}

fn ax_tree_scope_root(document: &NativeDom, node_id: NodeId) -> NodeId {
    let mut root = node_id;
    while let Some(parent) = document.parent_node(root) {
        root = parent;
    }
    root
}

fn ax_element_by_id(document: &NativeDom, root: NodeId, id: &str) -> Option<NodeId> {
    if id.is_empty() {
        return None;
    }

    let mut stack = vec![root];
    while let Some(node_id) = stack.pop() {
        if document
            .node(node_id)
            .and_then(Node::as_element)
            .is_some_and(|element| element.id() == Some(id))
        {
            return Some(node_id);
        }
        stack.extend(document.child_ids_reversed(node_id));
    }
    None
}

fn normalize_ax_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ax_document_name(document: &NativeDom) -> String {
    let title = document
        .document_head_handle()
        .and_then(|head| first_direct_html_child(document, head, "title"))
        .and_then(|title| document.text_content(title))
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty());
    title
        .or_else(|| document.final_url().map(|url| url.to_string()))
        .unwrap_or_default()
}

fn first_direct_html_child(
    document: &NativeDom,
    parent: NodeId,
    local_name: &str,
) -> Option<NodeId> {
    document.find_child(parent, |handle| {
        document
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.is_html_element(local_name))
    })
}

pub(super) fn ax_value(document: &NativeDom, node_id: NodeId, node: &Node) -> Option<Value> {
    let element = node.as_element()?;
    let value = match element.local_name() {
        "input"
            if !matches!(
                element.input_type(),
                InputType::Button
                    | InputType::Checkbox
                    | InputType::Hidden
                    | InputType::Image
                    | InputType::Radio
                    | InputType::Reset
                    | InputType::Submit
            ) =>
        {
            element.input_value()
        }
        "textarea" => {
            if element.input_value_dirty() {
                element.input_value()
            } else {
                node.direct_text_content(document)
            }
        }
        "select" => element.select_value(document, node_id, |child| {
            ax_option_is_selected(document, child)
        }),
        _ => String::new(),
    };
    if value.is_empty() {
        return None;
    }
    Some(json!({
        "type": "string",
        "value": value,
    }))
}

pub(super) fn ax_properties(document: &NativeDom, node_id: NodeId, node: &Node) -> Vec<Value> {
    let mut properties = Vec::new();
    match node.kind() {
        NodeData::Document(_) => {
            if let Some(url) = document.final_url() {
                properties.push(ax_string_property("url", url.as_str()));
            }
            properties.push(ax_bool_property("focusable", "booleanOrUndefined", true));
            properties.push(ax_bool_property("focused", "booleanOrUndefined", true));
        }
        NodeData::Element(element) => {
            if ax_role(node) == "status" {
                let live = element
                    .attribute("aria-live")
                    .map(str::trim)
                    .filter(|value| {
                        matches_ignore_ascii_case(value, &["off", "polite", "assertive"])
                    })
                    .unwrap_or("polite");
                properties.push(ax_token_property("live", live));

                let atomic = element
                    .attribute("aria-atomic")
                    .map(str::trim)
                    .and_then(|value| {
                        if value.eq_ignore_ascii_case("true") {
                            Some(true)
                        } else if value.eq_ignore_ascii_case("false") {
                            Some(false)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(true);
                properties.push(ax_bool_property("atomic", "boolean", atomic));

                let relevant = element
                    .attribute("aria-relevant")
                    .map(normalize_ax_token_list)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "additions text".to_owned());
                properties.push(ax_token_list_property("relevant", &relevant));
            }
            if let Some(level) = heading_level(element.local_name()) {
                properties.push(ax_integer_property("level", level));
            }
            if element.is_html_element("li") {
                use super::ax_roles::listitem_level;
                properties.push(ax_integer_property("level", listitem_level(document, node)));
            }
            if element.is_html_element("a")
                && let Some(url) = element
                    .attribute("href")
                    .and_then(|href| ax_resolve_element_url(document, href))
            {
                properties.push(ax_string_property("url", url.as_str()));
                properties.push(ax_bool_property("focusable", "booleanOrUndefined", true));
            }
            if element.is_html_element("img")
                && let Some(url) = element
                    .attribute("src")
                    .and_then(|src| ax_resolve_element_url(document, src))
            {
                properties.push(ax_string_property("url", url.as_str()));
            }
            if element.is_html_element("button") {
                let disabled = ax_control_is_disabled(document, node_id, element);
                if disabled {
                    properties.push(ax_bool_property("disabled", "boolean", true));
                }
                properties.push(ax_token_property("invalid", "false"));
                if !disabled {
                    properties.push(ax_bool_property("focusable", "booleanOrUndefined", true));
                }
            }
            if element.is_html_element("hr") {
                properties.push(ax_bool_property("settable", "booleanOrUndefined", true));
                properties.push(ax_token_property("orientation", "horizontal"));
            }
            if element.is_html_element("input") {
                let input_type = element.input_type();
                match input_type {
                    InputType::Text
                    | InputType::Email
                    | InputType::Tel
                    | InputType::Url
                    | InputType::Search
                    | InputType::Password
                    | InputType::Number => {
                        let disabled = ax_control_is_disabled(document, node_id, element);
                        if disabled {
                            properties.push(ax_bool_property("disabled", "boolean", true));
                        }
                        properties.push(ax_token_property("invalid", "false"));
                        if !disabled {
                            properties.push(ax_bool_property(
                                "focusable",
                                "booleanOrUndefined",
                                true,
                            ));
                        }
                        properties.push(ax_token_property("editable", "plaintext"));
                        if !disabled {
                            properties.push(ax_bool_property(
                                "settable",
                                "booleanOrUndefined",
                                true,
                            ));
                        }
                        properties.push(ax_bool_property("multiline", "boolean", false));
                        properties.push(ax_bool_property(
                            "readonly",
                            "boolean",
                            element.has_attribute("readonly"),
                        ));
                        properties.push(ax_bool_property(
                            "required",
                            "boolean",
                            element.has_attribute("required"),
                        ));
                    }
                    InputType::Checkbox | InputType::Radio => {
                        let disabled = ax_control_is_disabled(document, node_id, element);
                        if disabled {
                            properties.push(ax_bool_property("disabled", "boolean", true));
                        }
                        properties.push(ax_token_property("invalid", "false"));
                        if !disabled {
                            properties.push(ax_bool_property(
                                "focusable",
                                "booleanOrUndefined",
                                true,
                            ));
                        }
                        properties.push(ax_tristate_property(
                            "checked",
                            if input_type == InputType::Checkbox && element.indeterminate() {
                                "mixed"
                            } else if element.checked() {
                                "true"
                            } else {
                                "false"
                            },
                        ));
                    }
                    InputType::Button | InputType::Submit | InputType::Reset | InputType::Image => {
                        let disabled = ax_control_is_disabled(document, node_id, element);
                        if disabled {
                            properties.push(ax_bool_property("disabled", "boolean", true));
                        }
                        properties.push(ax_token_property("invalid", "false"));
                        if !disabled {
                            properties.push(ax_bool_property(
                                "focusable",
                                "booleanOrUndefined",
                                true,
                            ));
                        }
                    }
                    _ => {}
                }
            }
            if element.is_html_element("textarea") {
                let disabled = ax_control_is_disabled(document, node_id, element);
                if disabled {
                    properties.push(ax_bool_property("disabled", "boolean", true));
                }
                properties.push(ax_token_property("invalid", "false"));
                if !disabled {
                    properties.push(ax_bool_property("focusable", "booleanOrUndefined", true));
                }
                properties.push(ax_token_property("editable", "plaintext"));
                if !disabled {
                    properties.push(ax_bool_property("settable", "booleanOrUndefined", true));
                }
                properties.push(ax_bool_property("multiline", "boolean", true));
                properties.push(ax_bool_property(
                    "readonly",
                    "boolean",
                    element.has_attribute("readonly"),
                ));
                properties.push(ax_bool_property(
                    "required",
                    "boolean",
                    element.has_attribute("required"),
                ));
            }
            if element.is_html_element("select") {
                let disabled = ax_control_is_disabled(document, node_id, element);
                if disabled {
                    properties.push(ax_bool_property("disabled", "boolean", true));
                }
                properties.push(ax_token_property("invalid", "false"));
                if !disabled {
                    properties.push(ax_bool_property("focusable", "booleanOrUndefined", true));
                }
                properties.push(ax_token_property("hasPopup", "menu"));
                properties.push(ax_bool_property("expanded", "booleanOrUndefined", false));
            }
            if element.is_html_element("option") {
                properties.push(ax_bool_property("focusable", "booleanOrUndefined", true));
                if ax_option_is_selected(document, node_id) {
                    properties.push(ax_bool_property("selected", "booleanOrUndefined", true));
                }
            }
        }
        NodeData::DocumentType(_)
        | NodeData::Text(_)
        | NodeData::CDataSection(_)
        | NodeData::Comment(_)
        | NodeData::ProcessingInstruction(_)
        | NodeData::DocumentFragment(_) => {}
    }
    properties
}

fn ax_control_is_disabled(document: &NativeDom, node_id: NodeId, element: &Element) -> bool {
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

fn ax_option_is_selected(document: &NativeDom, option_id: NodeId) -> bool {
    let Some(option_node) = document.node(option_id) else {
        return false;
    };
    let Some(option) = option_node.as_element() else {
        return false;
    };
    if !option.is_html_option() {
        return false;
    }
    if option.selected() {
        return true;
    }

    let Some(parent_id) = option_node.parent_node_id() else {
        return false;
    };
    let Some(parent) = document.node(parent_id).and_then(Node::as_element) else {
        return false;
    };
    if !parent.is_html_select() || parent.has_attribute("multiple") {
        return false;
    }

    let option_ids = document
        .child_element_nodes(parent_id)
        .into_iter()
        .filter(|child_id| {
            document
                .node(*child_id)
                .and_then(Node::as_element)
                .is_some_and(|element| element.is_html_option())
        })
        .collect::<Vec<_>>();

    if option_ids.iter().any(|child_id| {
        document
            .node(*child_id)
            .and_then(Node::as_element)
            .is_some_and(|element| element.selected())
    }) {
        return false;
    }

    option_ids
        .into_iter()
        .find(|child_id| {
            document
                .node(*child_id)
                .and_then(Node::as_element)
                .is_some_and(|element| !element.has_attribute("disabled"))
        })
        .is_some_and(|selected_id| selected_id == option_id)
}

pub(super) fn ax_string_property(name: &str, value: &str) -> Value {
    json!({
        "name": name,
        "value": {
            "type": "string",
            "value": value,
        }
    })
}

pub(super) fn ax_bool_property(name: &str, kind: &str, value: bool) -> Value {
    json!({
        "name": name,
        "value": {
            "type": kind,
            "value": value,
        }
    })
}

pub(super) fn ax_token_property(name: &str, value: &str) -> Value {
    json!({
        "name": name,
        "value": {
            "type": "token",
            "value": value,
        }
    })
}

fn ax_token_list_property(name: &str, value: &str) -> Value {
    json!({
        "name": name,
        "value": {
            "type": "tokenList",
            "value": value,
        }
    })
}

fn matches_ignore_ascii_case(value: &str, expected: &[&str]) -> bool {
    expected
        .iter()
        .any(|expected| value.eq_ignore_ascii_case(expected))
}

fn normalize_ax_token_list(value: &str) -> String {
    value
        .split_whitespace()
        .filter(|token| matches_ignore_ascii_case(token, &["additions", "removals", "text", "all"]))
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .join(" ")
}

fn ax_tristate_property(name: &str, value: &str) -> Value {
    json!({
        "name": name,
        "value": {
            "type": "tristate",
            "value": value,
        }
    })
}

pub(super) fn ax_integer_property(name: &str, value: usize) -> Value {
    json!({
        "name": name,
        "value": {
            "type": "integer",
            "value": value,
        }
    })
}

fn ax_resolve_element_url(document: &NativeDom, value: &str) -> Option<Url> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let base = document.document().map(|doc| doc.base_url().clone());
    base.as_ref()
        .and_then(|base| Url::options().base_url(Some(base)).parse(value).ok())
        .or_else(|| Url::parse(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document_with_root() -> (NativeDom, NodeId) {
        let mut document =
            NativeDom::new_html(Url::parse("https://example.test/").expect("valid document URL"));
        let root = document.create_element("div");
        assert!(document.append_child(document.document_node_id(), root));
        (document, root)
    }

    fn append_text(document: &mut NativeDom, parent: NodeId, text: &str) {
        let text = document.create_text_node(text);
        assert!(document.append_child(parent, text));
    }

    fn node_name(document: &NativeDom, node_id: NodeId) -> String {
        ax_name(document, document.node(node_id).expect("named node"))
    }

    #[test]
    fn aria_labelledby_reference_uses_target_contents_not_its_labelledby() {
        let (mut document, root) = document_with_root();

        let button = document.create_element("button");
        assert!(document.set_attribute(button, "aria-labelledby", "middle"));
        append_text(&mut document, button, "Fallback");
        assert!(document.append_child(root, button));

        let middle = document.create_element("span");
        assert!(document.set_attribute(middle, "id", "middle"));
        assert!(document.set_attribute(middle, "aria-labelledby", "end"));
        append_text(&mut document, middle, "Middle");
        assert!(document.append_child(root, middle));

        let end = document.create_element("span");
        assert!(document.set_attribute(end, "id", "end"));
        append_text(&mut document, end, "End");
        assert!(document.append_child(root, end));

        assert_eq!(node_name(&document, button), "Middle");
    }

    #[test]
    fn valid_empty_aria_labelledby_target_does_not_fall_back_to_contents() {
        let (mut document, root) = document_with_root();

        let button = document.create_element("button");
        assert!(document.set_attribute(button, "aria-labelledby", "empty"));
        append_text(&mut document, button, "Fallback");
        assert!(document.append_child(root, button));

        let empty = document.create_element("span");
        assert!(document.set_attribute(empty, "id", "empty"));
        assert!(document.append_child(root, empty));

        assert_eq!(node_name(&document, button), "");
    }

    fn alternating_relation_chain_name(pair_count: usize) -> String {
        let (mut document, root) = document_with_root();

        let button = document.create_element("button");
        assert!(document.set_attribute(button, "aria-labelledby", "input0"));
        append_text(&mut document, button, "Fallback");
        assert!(document.append_child(root, button));

        for index in 0..pair_count {
            let input_id = format!("input{index}");
            let target_id = if index + 1 < pair_count {
                format!("input{}", index + 1)
            } else {
                "end".to_owned()
            };

            let label = document.create_element("label");
            assert!(document.set_attribute(label, "for", &input_id));
            assert!(document.set_attribute(label, "aria-labelledby", &target_id));
            assert!(document.append_child(root, label));

            let input = document.create_element("input");
            assert!(document.set_attribute(input, "id", &input_id));
            assert!(document.append_child(root, input));
        }

        let end = document.create_element("span");
        assert!(document.set_attribute(end, "id", "end"));
        append_text(&mut document, end, "End");
        assert!(document.append_child(root, end));

        node_name(&document, button)
    }

    #[test]
    fn accessible_name_relation_budget_matches_chromium_boundary() {
        assert_eq!(alternating_relation_chain_name(49), "End");
        assert_eq!(alternating_relation_chain_name(50), "");
    }
}
