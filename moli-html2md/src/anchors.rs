use std::collections::HashSet;

use crate::{Dom, NodeKind};

pub(crate) fn referenced<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    limit: usize,
) -> HashSet<String> {
    let mut ids = HashSet::new();
    if !dom.may_have_fragment_links() {
        return ids;
    }
    walk(dom, root, limit, |node| {
        if let Some(fragment) = dom
            .attribute(node, "href")
            .and_then(|href| href.strip_prefix('#'))
            && !fragment.is_empty()
        {
            ids.insert(decode_fragment(fragment));
        }
        false
    });
    ids
}

pub(crate) fn contains<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    limit: usize,
    ids: &HashSet<String>,
) -> bool {
    walk(dom, root, limit, |node| {
        ["id", "name"]
            .iter()
            .any(|name| dom.attribute(node, name).is_some_and(|id| ids.contains(id)))
    })
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
