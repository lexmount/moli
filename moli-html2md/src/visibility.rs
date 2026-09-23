use crate::{Dom, NodeKind};

/// A hidden leaf containing a serialized page-state object is not reader text.
/// Keep other hidden subtrees: inactive tabs and collapsed details can contain
/// the page's primary data and belong in a structural content dump.
pub(crate) fn nonrendered_serialized_state<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> bool {
    if !matches!(dom.node_kind(node), NodeKind::Element(_)) {
        return false;
    }
    let hidden = dom.attribute(node, "hidden").is_some()
        || dom
            .attribute(node, "style")
            .is_some_and(inline_display_none);
    if !hidden {
        return false;
    }
    let Some(child) = dom.first_child(node) else {
        return false;
    };
    if dom.next_sibling(child).is_some() {
        return false;
    }
    let NodeKind::Text(text) = dom.node_kind(child) else {
        return false;
    };
    let trimmed = text.trim();
    trimmed.len() >= 512
        && ((trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
}

fn inline_display_none(style: &str) -> bool {
    let mut display = None;
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        if !property.trim().eq_ignore_ascii_case("display") {
            continue;
        }
        let value = value.trim().to_ascii_lowercase();
        let important = value.trim_end().ends_with("!important");
        let value = value.trim_end_matches("!important").trim();
        if !display.is_some_and(|(_, previous_important)| previous_important && !important) {
            display = Some((value == "none", important));
        }
    }
    display.is_some_and(|(hidden, _)| hidden)
}
