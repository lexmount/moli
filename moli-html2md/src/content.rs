use crate::{Dom, NodeKind};

pub(crate) fn has_readable_content<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    limit: usize,
) -> bool {
    let mut pending = vec![(root, 0)];
    while let Some((node, depth)) = pending.pop() {
        if matches!(
            dom.node_kind(node),
            NodeKind::Element("head" | "title" | "script" | "style" | "noscript" | "template")
        ) {
            continue;
        }
        match dom.node_kind(node) {
            NodeKind::Text(text) if !text.trim_matches(char::is_whitespace).is_empty() => {
                return true;
            }
            NodeKind::Element("img")
                if crate::media::source(dom, node).is_some()
                    || dom
                        .attribute(node, "alt")
                        .is_some_and(|alt| !alt.trim().is_empty()) =>
            {
                return true;
            }
            NodeKind::Element("input") if crate::form::input_text(dom, node).is_some() => {
                return true;
            }
            NodeKind::Element(_) if depth > 0 && fallback_text(dom, node).is_some() => return true,
            _ => {}
        }
        if depth + 1 >= limit {
            continue;
        }
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            pending.push((id, depth + 1));
            child = dom.next_sibling(id);
        }
    }
    false
}

pub(crate) fn fallback_text<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> Option<String> {
    for name in ["aria-label", "data-original-title", "title"] {
        if let Some(value) = dom
            .attribute(node, name)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_owned());
        }
    }
    if matches!(dom.node_kind(node), NodeKind::Element("svg")) {
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
