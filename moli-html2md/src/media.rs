use std::borrow::Cow;

use crate::{Dom, NodeKind};

/// Frozen DOMs can still carry lazy-loading placeholders. These conventional
/// attributes declare the resource the page intends to load, not the spacer.
pub(crate) fn source<'a, D: Dom + ?Sized>(dom: &'a D, node: D::NodeId) -> Option<Cow<'a, str>> {
    if let Some(value) = direct_source(dom, node) {
        return Some(Cow::Borrowed(value));
    }

    // A responsive image may intentionally leave `src` empty. Retain the
    // largest declared candidate so the image does not disappear entirely.
    dom.attribute(node, "srcset")
        .and_then(largest_srcset_candidate)
        .filter(|value| safe_url(value, true))
        .map(|value| Cow::Owned(value.to_owned()))
}

fn direct_source<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> Option<&str> {
    let usable = |value: &&str| {
        !value.is_empty()
            && !value.starts_with('#')
            && *value != "about:blank"
            && safe_url(value, true)
    };
    let attr = |name| dom.attribute(node, name).map(str::trim).filter(usable);

    if let Some(value) = attr("data-original") {
        return Some(value);
    }
    let src = attr("src");
    if let Some(value) = src.filter(|value| !is_placeholder_source(value)) {
        return Some(value);
    }
    attr("data-src").or_else(|| attr("data-lazy-src")).or(src)
}

fn is_placeholder_source(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("data:image/")
        || [
            "placeholder",
            "spacer",
            "transparent",
            "blank.gif",
            "blank.png",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn largest_srcset_candidate(srcset: &str) -> Option<&str> {
    srcset
        .split(',')
        .filter_map(|candidate| candidate.split_ascii_whitespace().next())
        .rfind(|url| !url.is_empty())
}

pub(crate) fn safe_url(value: &str, image: bool) -> bool {
    let normalized: String = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_ascii_control())
        .collect();
    let Some((scheme, _)) = normalized.split_once(':') else {
        return true;
    };
    if scheme.contains(['/', '#', '?']) {
        return true;
    }
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "mailto" | "tel" | "ftp"
    ) || (image && normalized.to_ascii_lowercase().starts_with("data:image/"))
}

pub(crate) fn sources<'a, D: Dom + ?Sized>(
    dom: &'a D,
    node: D::NodeId,
    tag: &str,
    depth: usize,
    limit: usize,
) -> Vec<&'a str> {
    let mut urls = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(src) = direct_source(dom, node) {
        seen.insert(src);
        urls.push(src);
    }
    if tag != "iframe" && depth + 1 < limit {
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            child = dom.next_sibling(id);
            if dom.node_kind(id) == NodeKind::Element("source")
                && let Some(src) = direct_source(dom, id)
                && seen.insert(src)
            {
                urls.push(src);
            }
        }
    }
    urls
}

pub(crate) fn label<'a, D: Dom + ?Sized>(dom: &'a D, node: D::NodeId, tag: &str) -> &'a str {
    dom.attribute(node, "title")
        .filter(|label| !label.trim().is_empty())
        .unwrap_or(match tag {
            "video" => "Video",
            "audio" => "Audio",
            _ => "Embedded content",
        })
}
