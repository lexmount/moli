use crate::{Dom, NodeKind};

/// Private lazy-loading conventions, used only when standard sources are absent.
const LAZY_SOURCE_ATTRIBUTES: [&str; 3] = ["data-src", "data-lazy-src", "data-original"];

/// URL-bearing inputs consumed by Markdown conversion. Browser adapters use
/// this inventory to resolve the same inputs against the document base URL.
#[doc(hidden)]
pub fn resource_url_attributes(tag: &str) -> impl Iterator<Item = &'static str> {
    let standard: &[&str] = match tag {
        "a" => &["href"],
        "img" | "source" => &["src", "srcset", "data-srcset"],
        "video" => &["src", "poster"],
        "audio" | "iframe" => &["src"],
        _ => &[],
    };
    let lazy_count = if matches!(tag, "img" | "source" | "video" | "audio" | "iframe") {
        LAZY_SOURCE_ATTRIBUTES.len()
    } else {
        0
    };
    standard
        .iter()
        .copied()
        .chain(LAZY_SOURCE_ATTRIBUTES.into_iter().take(lazy_count))
}

/// Frozen DOMs can still carry lazy-loading placeholders. These conventional
/// attributes declare the resource the page intends to load, not the spacer.
pub(crate) fn source<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> Option<&str> {
    // A responsive declaration determines the rendered resource even when a
    // fallback `src` is present. Private lazy-load attributes remain fallback
    // inputs and never override a valid standard declaration.
    dom.attribute(node, "srcset")
        .and_then(largest_srcset_candidate)
        .or_else(|| direct_attribute(dom, node, "src", true))
        .or_else(|| {
            dom.attribute(node, "data-srcset")
                .and_then(largest_srcset_candidate)
        })
        .or_else(|| {
            LAZY_SOURCE_ATTRIBUTES
                .into_iter()
                .find_map(|name| direct_attribute(dom, node, name, true))
        })
}

fn direct_source<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> Option<&str> {
    direct_attribute(dom, node, "src", false).or_else(|| {
        LAZY_SOURCE_ATTRIBUTES
            .into_iter()
            .find_map(|name| direct_attribute(dom, node, name, false))
    })
}

pub(crate) fn direct_attribute<'a, D: Dom + ?Sized>(
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

/// A validated HTML responsive-image candidate. Dimensions are descriptor
/// values, not an image-selection policy; consumers choose using their viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SrcsetCandidate<'a> {
    pub url: &'a str,
    pub descriptor: &'a str,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub density: Option<f64>,
}

/// Parse HTML srcset syntax, discarding malformed candidates.
/// Non-ASCII spaces are resource/descriptor characters, not separators.
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
                    width: None,
                    height: None,
                    density: None,
                });
            }
            continue;
        }

        let descriptor_start = offset;
        let mut in_parentheses = false;
        while offset < bytes.len() {
            match bytes[offset] {
                b'(' => in_parentheses = true,
                b')' => in_parentheses = false,
                b',' if !in_parentheses => break,
                _ => {}
            }
            offset += 1;
        }
        let descriptor = srcset[descriptor_start..offset].trim_ascii();
        let parsed = descriptor_values(descriptor);
        if let Some((width, height, density)) = parsed
            && !url.is_empty()
        {
            candidates.push(SrcsetCandidate {
                url,
                descriptor,
                width,
                height,
                density,
            });
        }
    }
    candidates
}

fn descriptor_values(descriptor: &str) -> Option<(Option<u64>, Option<u64>, Option<f64>)> {
    if descriptor.is_empty() {
        return Some((None, None, None));
    }
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
        (Some(_), _, None) | (None, None, Some(_)) => Some((width, height, density)),
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

fn largest_srcset_candidate(srcset: &str) -> Option<&str> {
    parse_srcset(srcset)
        .into_iter()
        .filter(|candidate| safe_url(candidate.url, true))
        .max_by(|left, right| {
            let score = |candidate: &SrcsetCandidate<'_>| {
                candidate
                    .width
                    .map(|v| v as f64)
                    .or(candidate.density)
                    .unwrap_or(1.0)
            };
            score(left).total_cmp(&score(right))
        })
        .map(|candidate| candidate.url)
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
        "http" | "https" | "mailto" | "tel" | "ftp" | "file"
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
    dom.attribute(node, "aria-label")
        .filter(|label| !label.trim().is_empty())
        .or_else(|| {
            dom.attribute(node, "title")
                .filter(|label| !label.trim().is_empty())
        })
        .unwrap_or(match tag {
            "video" => "Video",
            "audio" => "Audio",
            _ => "Embedded content",
        })
}

/// Use the same nonempty, acceptable destination in both anchor output paths.
pub(crate) fn link_href<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> Option<&str> {
    dom.attribute(node, "href")
        .filter(|href| !href.is_empty() && safe_link_url(href))
}

/// Links preserve application protocols; executable inline-document schemes do not.
pub(crate) fn safe_link_url(value: &str) -> bool {
    let normalized: String = value
        .trim()
        .chars()
        .filter(|c| !c.is_ascii_control())
        .collect();
    let normalized = normalized.to_ascii_lowercase();
    !normalized.starts_with("javascript:")
        && !normalized.starts_with("vbscript:")
        && !normalized.starts_with("data:text/html")
        && !normalized.starts_with("data:application/xhtml+xml")
}

pub(crate) fn poster_label<D: Dom + ?Sized>(dom: &D, node: D::NodeId) -> String {
    format!("{} poster", label(dom, node, "video"))
}

pub(crate) fn poster<'a, D: Dom + ?Sized>(
    dom: &'a D,
    node: D::NodeId,
    tag: &str,
) -> Option<&'a str> {
    (tag == "video")
        .then(|| direct_attribute(dom, node, "poster", true))
        .flatten()
}
