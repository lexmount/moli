use crate::{Dom, NodeKind};

pub(crate) fn fallback_text<D: Dom + ?Sized>(
    dom: &D,
    node: D::NodeId,
    remaining_depth: usize,
) -> Option<String> {
    if excluded_from_naming(dom, node) {
        return None;
    }
    let named_tag = matches!(
        dom.node_kind(node),
        NodeKind::Element(
            "a" | "button"
                | "svg"
                | "select"
                | "option"
                | "optgroup"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
        )
    );
    let named_role =
        primary_role(dom, node).is_some_and(|role| role_matches(role, NAMED_WIDGET_ROLES));
    if !named_tag && !named_role {
        return None;
    }
    for name in ["aria-label", "title"] {
        if let Some(value) = dom
            .attribute(node, name)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_owned());
        }
    }
    // Title text is two levels below SVG and obeys the same depth limit as body text.
    if matches!(dom.node_kind(node), NodeKind::Element("svg")) && remaining_depth > 2 {
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            if dom.node_kind(id) == NodeKind::Element("title") {
                let mut text = String::new();
                let mut title_child = dom.first_child(id);
                while let Some(part) = title_child {
                    if let NodeKind::Text(value) = dom.node_kind(part) {
                        text.push_str(value);
                    }
                    title_child = dom.next_sibling(part);
                }
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
            child = dom.next_sibling(id);
        }
    }
    None
}

const PRESENTATIONAL_ROLES: &[&str] = &["none", "presentation"];
const NAMED_WIDGET_ROLES: &[&str] = &[
    "button", "link", "img", "heading", "checkbox", "radio", "textbox", "combobox", "switch", "tab",
];

// This structural converter uses the first authored token as a conservative
// naming hint; it does not implement an accessibility-tree role resolver.
fn primary_role<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> Option<&str> {
    dom.attribute(node, "role")?.split_ascii_whitespace().next()
}
fn role_matches(role: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| role.eq_ignore_ascii_case(candidate))
}

fn excluded_from_naming<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> bool {
    dom.attribute(node, "aria-hidden")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
        || dom.attribute(node, "hidden").is_some()
        || primary_role(dom, node).is_some_and(|role| role_matches(role, PRESENTATIONAL_ROLES))
}

pub(crate) fn image_label<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> &str {
    let alt = dom.attribute(node, "alt");
    if let Some(alt) = alt.filter(|alt| !alt.is_empty()) {
        return alt;
    }
    if excluded_from_naming(dom, node) {
        return "";
    }
    ["aria-label", "title"]
        .into_iter()
        .filter(|name| *name != "title" || alt.is_none())
        .find_map(|name| {
            dom.attribute(node, name)
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_default()
}
