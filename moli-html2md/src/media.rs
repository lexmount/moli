use std::borrow::Cow;

use crate::{Dom, NodeKind};

/// Frozen DOMs can still carry lazy-loading placeholders. These conventional
/// attributes declare the resource the page intends to load, not the spacer.
pub(crate) fn source<'a, D: Dom + ?Sized>(dom: &'a D, node: D::NodeId) -> Option<Cow<'a, str>> {
    // A responsive declaration determines the rendered resource even when a
    // fallback `src` is present. Private lazy-load attributes remain fallback
    // inputs and never override a valid standard declaration.
    if let Some(value) = dom
        .attribute(node, "srcset")
        .and_then(|value| largest_srcset_candidate(value, true))
    {
        return Some(value);
    }
    let src = direct_attribute(dom, node, "src", true);
    if src.is_some_and(|value| !is_explicit_placeholder(value)) {
        return src.map(Cow::Borrowed);
    }
    if let Some(value) = dom
        .attribute(node, "data-srcset")
        .and_then(|value| largest_srcset_candidate(value, true))
    {
        return Some(value);
    }
    ["data-src", "data-lazy-src", "data-original"]
        .into_iter()
        .find_map(|name| direct_attribute(dom, node, name, true))
        .or(src)
        .map(Cow::Borrowed)
}

fn direct_source<D: Dom + ?Sized>(dom: &D, node: D::NodeId, image: bool) -> Option<&str> {
    let src = direct_attribute(dom, node, "src", image);
    if src.is_some_and(is_explicit_placeholder) {
        return direct_attribute(dom, node, "data-src", image)
            .or_else(|| direct_attribute(dom, node, "data-lazy-src", image))
            .or_else(|| direct_attribute(dom, node, "data-original", image))
            .or(src);
    }
    src.or_else(|| direct_attribute(dom, node, "data-src", image))
        .or_else(|| direct_attribute(dom, node, "data-lazy-src", image))
        .or_else(|| direct_attribute(dom, node, "data-original", image))
}

fn direct_attribute<'a, D: Dom + ?Sized>(
    dom: &'a D,
    node: D::NodeId,
    name: &str,
    image: bool,
) -> Option<&'a str> {
    dom.attribute(node, name).map(str::trim).filter(|value| {
        !value.is_empty()
            && !value.starts_with('#')
            && *value != "about:blank"
            && safe_url(value, image)
    })
}

fn is_explicit_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
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

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SrcsetCandidate<'a> {
    pub url: &'a str,
    pub descriptor: &'a str,
    pub score: f64,
}

#[doc(hidden)]
pub fn parse_srcset(srcset: &str) -> Vec<SrcsetCandidate<'_>> {
    let bytes = srcset.as_bytes();
    let mut offset = 0;
    let mut candidates = Vec::new();
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
        let ended_with_comma = url_end < offset;
        if ended_with_comma {
            if !url.is_empty() {
                candidates.push(SrcsetCandidate {
                    url,
                    descriptor: "",
                    score: 1.0,
                });
            }
            continue;
        }

        let descriptor_start = offset;
        let mut parentheses = 0usize;
        while offset < bytes.len() {
            match bytes[offset] {
                b'(' => parentheses += 1,
                b')' => parentheses = parentheses.saturating_sub(1),
                b',' if parentheses == 0 => break,
                _ => {}
            }
            offset += 1;
        }
        let descriptor = srcset[descriptor_start..offset].trim();
        let score = if descriptor.is_empty() {
            Some(1.0)
        } else {
            descriptor_score(descriptor)
        };
        if let Some(score) = score
            && !url.is_empty()
        {
            candidates.push(SrcsetCandidate {
                url,
                descriptor,
                score,
            });
        }
    }
    candidates
}

fn descriptor_score(descriptor: &str) -> Option<f64> {
    let mut width = None;
    let mut height = None;
    let mut density = None;
    for token in descriptor.split_ascii_whitespace() {
        if let Some(value) = token.strip_suffix('w') {
            if width.is_some() {
                return None;
            }
            width = Some(valid_positive_integer(value)?);
        } else if let Some(value) = token.strip_suffix('h') {
            if height.is_some() {
                return None;
            }
            height = Some(valid_positive_integer(value)?);
        } else if let Some(value) = token.strip_suffix('x') {
            if density.is_some() {
                return None;
            }
            density = Some(valid_positive_float(value)?);
        } else {
            return None;
        }
    }
    match (width, height, density) {
        (Some(width), _, None) => Some(width as f64),
        (None, None, Some(density)) => Some(density),
        _ => None,
    }
}

fn valid_positive_integer(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<u64>().ok())
        .flatten()
        .filter(|value| *value > 0)
}

fn valid_positive_float(value: &str) -> Option<f64> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes[0] == b'+' {
        return None;
    }
    let mut offset = usize::from(bytes[0] == b'-');
    if offset == bytes.len() {
        return None;
    }
    let integer_start = offset;
    while offset < bytes.len() && bytes[offset].is_ascii_digit() {
        offset += 1;
    }
    let has_integer = offset > integer_start;
    let mut has_fraction = false;
    if bytes.get(offset) == Some(&b'.') {
        offset += 1;
        let fraction = offset;
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            offset += 1;
        }
        if fraction == offset {
            return None;
        }
        has_fraction = true;
    }
    if !has_integer && !has_fraction {
        return None;
    }
    if matches!(bytes.get(offset), Some(b'e' | b'E')) {
        offset += 1;
        if matches!(bytes.get(offset), Some(b'+' | b'-')) {
            offset += 1;
        }
        let exponent = offset;
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            offset += 1;
        }
        if exponent == offset {
            return None;
        }
    }
    (offset == bytes.len())
        .then(|| value.parse::<f64>().ok())
        .flatten()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn largest_srcset_candidate(srcset: &str, image: bool) -> Option<Cow<'_, str>> {
    parse_srcset(srcset)
        .into_iter()
        .filter(|candidate| safe_url(candidate.url, image))
        .max_by(|left, right| left.score.total_cmp(&right.score))
        .map(|candidate| Cow::Owned(candidate.url.to_owned()))
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
