use std::collections::HashSet;

use crate::{Dom, NodeKind};

pub(crate) struct Targets {
    referenced: HashSet<String>,
    ids: HashSet<String>,
}

impl Targets {
    pub(crate) fn is_empty(&self) -> bool {
        self.referenced.is_empty()
    }

    pub(crate) fn contains(&self, value: &str) -> bool {
        self.referenced.contains(value)
    }

    pub(crate) fn targets<'a, D: Dom + ?Sized>(&self, dom: &'a D, node: D::NodeId) -> Vec<&'a str> {
        let mut targets = Vec::with_capacity(2);
        if let Some(id) = dom.attribute(node, "id").filter(|id| self.contains(id)) {
            targets.push(id);
        }
        if dom.node_kind(node) == NodeKind::Element("a")
            && let Some(name) = dom
                .attribute(node, "name")
                .filter(|name| self.contains(name) && !self.ids.contains(*name))
            && !targets.contains(&name)
        {
            targets.push(name);
        }
        targets
    }
}

pub(crate) fn referenced<D: Dom + ?Sized>(dom: &D, root: D::NodeId, limit: usize) -> Targets {
    let mut available = HashSet::new();
    let mut element_ids = HashSet::new();
    if !dom.may_have_fragment_links() {
        return Targets {
            referenced: available,
            ids: element_ids,
        };
    }
    walk(dom, root, limit, |node| {
        for attribute in ["id", "name"] {
            if attribute == "name" && dom.node_kind(node) != NodeKind::Element("a") {
                continue;
            }
            if let Some(value) = dom.attribute(node, attribute) {
                available.insert(value.to_owned());
                if attribute == "id" {
                    element_ids.insert(value.to_owned());
                }
            }
        }
        false
    });
    let mut referenced = HashSet::new();
    walk(dom, root, limit, |node| {
        if let Some(fragment) = dom
            .attribute(node, "href")
            .and_then(|href| href.strip_prefix('#'))
            && !fragment.is_empty()
        {
            // HTML fragment navigation tries the literal fragment first and
            // only percent-decodes it when no literal target exists.
            if available.contains(fragment) {
                referenced.insert(fragment.to_owned());
            } else {
                let decoded = decode_fragment(fragment);
                if available.contains(&decoded) {
                    referenced.insert(decoded);
                }
            }
        }
        false
    });
    Targets {
        referenced,
        ids: element_ids,
    }
}

pub(crate) fn contains<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    limit: usize,
    ids: &Targets,
) -> bool {
    walk(dom, root, limit, |node| !ids.targets(dom, node).is_empty())
}

pub(crate) fn within<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    limit: usize,
    ids: &Targets,
) -> Vec<String> {
    let mut found = Vec::new();
    walk(dom, root, limit, |node| {
        for id in ids.targets(dom, node) {
            if !found.iter().any(|existing| existing == id) {
                found.push(id.to_owned());
            }
        }
        false
    });
    found
}

fn walk<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    limit: usize,
    mut found: impl FnMut(D::NodeId) -> bool,
) -> bool {
    let mut stack = vec![(Some(root), 0, false)];
    while let Some((node, depth, siblings)) = stack.pop() {
        let Some(node) = node else {
            continue;
        };
        if siblings {
            stack.push((dom.next_sibling(node), depth, true));
        }
        if depth >= limit {
            continue;
        }
        if matches!(
            dom.node_kind(node),
            NodeKind::Other
                | NodeKind::Element("head" | "script" | "style" | "noscript" | "template")
        ) {
            continue;
        }
        if found(node) {
            return true;
        }
        stack.push((dom.first_child(node), depth + 1, true));
    }
    false
}

pub(crate) fn decode_fragment(fragment: &str) -> String {
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'%' && offset + 2 < bytes.len() {
            let hex = |byte: u8| (byte as char).to_digit(16);
            if let (Some(high), Some(low)) = (hex(bytes[offset + 1]), hex(bytes[offset + 2])) {
                decoded.push((high * 16 + low) as u8);
                offset += 3;
                continue;
            }
        }
        decoded.push(bytes[offset]);
        offset += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| fragment.to_owned())
}

pub(crate) fn markup(id: &str) -> String {
    let mut result = String::from("<a id=\"");
    for character in id.chars() {
        match character {
            '&' => result.push_str("&amp;"),
            '"' => result.push_str("&quot;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '\n' => result.push_str("&#10;"),
            '\r' => result.push_str("&#13;"),
            character => result.push(character),
        }
    }
    result.push_str("\"></a>");
    result
}
