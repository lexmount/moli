use std::borrow::Cow;

use crate::{Dom, NodeKind};

/// Frozen DOMs can still carry lazy-loading placeholders. These conventional
/// attributes declare the resource the page intends to load, not the spacer.
pub(crate) fn source<'a, D: Dom + ?Sized>(dom: &'a D, node: D::NodeId) -> Option<Cow<'a, str>> {
    // A responsive declaration determines the rendered resource even when a
    // fallback `src` is present. Private lazy-load attributes remain fallback
    // inputs and never override a valid standard declaration.
    if let Some(value) = ["srcset", "data-srcset"].into_iter().find_map(|name| {
        dom.attribute(node, name)
            .and_then(largest_srcset_candidate)
            .filter(|value| safe_url(value, true))
            .map(|value| Cow::Owned(value.to_owned()))
    }) {
        return Some(value);
    }
    direct_source(dom, node, true).map(Cow::Borrowed)
}

fn direct_source<D: Dom + ?Sized>(dom: &D, node: D::NodeId, image: bool) -> Option<&str> {
    let usable = |value: &&str| {
        !value.is_empty()
            && !value.starts_with('#')
            && *value != "about:blank"
            && safe_url(value, image)
    };
    let attr = |name| dom.attribute(node, name).map(str::trim).filter(usable);

    let src = attr("src");
    if src.is_some_and(is_explicit_placeholder) {
        return attr("data-src")
            .or_else(|| attr("data-lazy-src"))
            .or_else(|| attr("data-original"))
            .or(src);
    }
    src.or_else(|| attr("data-src"))
        .or_else(|| attr("data-lazy-src"))
        .or_else(|| attr("data-original"))
}

fn is_explicit_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("data:image/") {
        return true;
    }
    let path = lower
        .split(['?', '#'])
        .next()
        .unwrap_or(&lower)
        .trim_end_matches('/');
    matches!(
        path.rsplit('/').next().unwrap_or_default(),
        "placeholder.gif"
            | "placeholder.png"
            | "spacer.gif"
            | "spacer.png"
            | "transparent.gif"
            | "transparent.png"
            | "blank.gif"
            | "blank.png"
    )
}

fn largest_srcset_candidate(srcset: &str) -> Option<&str> {
    let bytes = srcset.as_bytes();
    let mut offset = 0;
    let mut best: Option<(&str, f64)> = None;
    while offset < bytes.len() {
        while offset < bytes.len() && (bytes[offset].is_ascii_whitespace() || bytes[offset] == b',')
        {
            offset += 1;
        }
        let url_start = offset;
        while offset < bytes.len() && !bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        if url_start == offset {
            break;
        }
        let mut url_end = offset;
        while url_end > url_start && bytes[url_end - 1] == b',' {
            url_end -= 1;
        }
        let url = &srcset[url_start..url_end];
        let descriptor_start = offset;
        while offset < bytes.len() && bytes[offset] != b',' {
            offset += 1;
        }
        let descriptors = srcset[descriptor_start..offset].split_ascii_whitespace();
        let mut score = 1.0;
        let mut valid = true;
        let mut seen = false;
        for descriptor in descriptors {
            if seen {
                valid = false;
                break;
            }
            seen = true;
            score = if let Some(value) = descriptor.strip_suffix('w') {
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .map(f64::from)
            } else if let Some(value) = descriptor.strip_suffix('x') {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && *value > 0.0)
            } else {
                None
            }
            .unwrap_or_else(|| {
                valid = false;
                0.0
            });
        }
        if valid && !url.is_empty() && best.is_none_or(|(_, current)| score >= current) {
            best = Some((url, score));
        }
    }
    best.map(|(url, _)| url)
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
    if let Some(src) = direct_source(dom, node, tag == "img") {
        seen.insert(src);
        urls.push(src);
    }
    if tag != "iframe" && depth + 1 < limit {
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            child = dom.next_sibling(id);
            if dom.node_kind(id) == NodeKind::Element("source")
                && let Some(src) = direct_source(dom, id, false)
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
