use std::collections::{HashMap, HashSet};

use moli_html2md::{Dom, NodeKind};
use url::Url;

use crate::dom::native::{NativeDom, NativeNodeId};

/// Read the current page's computed visibility without mutating the live DOM.
/// Explicit disclosure content remains useful in a document dump, even while
/// its tab or section is closed. Other non-rendered UI is not document text.
pub(super) struct MarkdownDom<'a> {
    dom: &'a NativeDom,
    suppressed: HashSet<NativeNodeId>,
    invisible: HashSet<NativeNodeId>,
    blocks: HashSet<NativeNodeId>,
    inline_boundaries: HashSet<NativeNodeId>,
    superscripts: HashSet<NativeNodeId>,
    subscripts: HashSet<NativeNodeId>,
    resolved_urls: HashMap<NativeNodeId, HashMap<String, String>>,
    fragment_links: bool,
}

impl<'a> MarkdownDom<'a> {
    pub(super) fn new(
        dom: &'a NativeDom,
        styles: impl IntoIterator<Item = (NativeNodeId, Vec<String>)>,
        base_url: Option<&Url>,
    ) -> Self {
        let styles: Vec<_> = styles.into_iter().collect();
        let document_element = dom.document_element_node_id();
        let body = dom.body_node_id();
        let document_visibility_is_hidden = styles.iter().any(|(node, values)| {
            (Some(*node) == document_element || Some(*node) == body)
                && values
                    .get(1)
                    .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse"))
        });
        let fragment_links = styles.iter().any(|(node, _)| {
            Dom::attribute(dom, *node, "href").is_some_and(|href| href.starts_with('#'))
        });
        // `styles` follows the document's preorder, so every element's parent
        // has already been seen. Cache the painted ancestor background once;
        // walking ancestors for every node is quadratic on deeply nested pages.
        let mut effective_backgrounds = HashMap::new();
        if styles
            .iter()
            .any(|(_, values)| values.get(11).is_some_and(|value| !value.is_empty()))
        {
            for (node, values) in &styles {
                let own = values
                    .get(11)
                    .and_then(|value| css_color(value))
                    .filter(|color| color.3 > 0.99);
                let inherited = dom
                    .parent_node(*node)
                    .and_then(|parent| effective_backgrounds.get(&parent).copied());
                effective_backgrounds
                    .insert(*node, own.or(inherited).unwrap_or((255, 255, 255, 1.0)));
            }
        }
        let mut disclosures = HashSet::new();
        let mut excerpts = HashSet::new();
        let targets: HashMap<_, _> = styles
            .iter()
            .filter_map(|(node, values)| {
                (values.first().is_some_and(|value| value == "none"))
                    .then(|| Dom::attribute(dom, *node, "id").map(|id| (id, *node)))
                    .flatten()
            })
            .collect();
        for (node, _) in &styles {
            if Dom::attribute(dom, *node, "aria-expanded").is_some()
                || Dom::attribute(dom, *node, "role")
                    .is_some_and(|role| role.eq_ignore_ascii_case("tab"))
            {
                if let Some(targets) = Dom::attribute(dom, *node, "aria-controls") {
                    disclosures.extend(targets.split_ascii_whitespace());
                }
            }
            // Some pages pair a shortened paragraph with an explicitly linked
            // hidden full-text copy. Keep the complete copy once; an unrelated
            // dialog or a matching paragraph elsewhere is not such a pair.
            for attribute in ["href", "data-src", "data-target"] {
                let Some(id) =
                    Dom::attribute(dom, *node, attribute).and_then(|value| value.strip_prefix('#'))
                else {
                    continue;
                };
                let Some(&target) = targets.get(id) else {
                    continue;
                };
                if Dom::attribute(dom, target, "role").is_some_and(|role| {
                    ["dialog", "alertdialog", "menu"]
                        .iter()
                        .any(|excluded| role.eq_ignore_ascii_case(excluded))
                }) {
                    continue;
                }
                let mut ancestor = dom.parent_node(*node);
                while let Some(paragraph) = ancestor {
                    if matches!(Dom::node_kind(dom, paragraph), NodeKind::Element("p")) {
                        if let Some(container) = dom.parent_node(paragraph) {
                            if within(dom, target, container) && !within(dom, target, paragraph) {
                                let short = text(dom, paragraph, Some(*node));
                                let prefix = short
                                    .strip_suffix("...")
                                    .or_else(|| short.strip_suffix('…'));
                                if let Some(prefix) = prefix
                                    .map(str::trim_end)
                                    .filter(|prefix| !prefix.is_empty())
                                {
                                    let full = text(dom, target, None);
                                    if full.len() > prefix.len() && full.starts_with(prefix) {
                                        disclosures.insert(id);
                                        excerpts.insert(paragraph);
                                    }
                                }
                            }
                        }
                        break;
                    }
                    ancestor = dom.parent_node(paragraph);
                }
            }
        }
        let mut suppressed = excerpts;
        let mut invisible = HashSet::new();
        let mut blocks = HashSet::new();
        let mut inline_boundaries = HashSet::new();
        let mut superscripts = HashSet::new();
        let mut subscripts = HashSet::new();
        let mut resolved_urls = HashMap::new();
        for (node, values) in styles {
            let role = Dom::attribute(dom, node, "role").unwrap_or_default();
            // Lazy media often stays transparent until a scroll/load event.
            // Its declared resource still belongs to the document, unlike
            // transparent numeric placeholders or hidden UI text.
            let lazy_media = matches!(
                Dom::node_kind(dom, node),
                NodeKind::Element("img" | "iframe" | "video" | "audio")
            ) && ["data-original", "data-src", "data-lazy-src"].iter().any(
                |attribute| {
                    Dom::attribute(dom, node, attribute)
                        .is_some_and(|value| !value.trim().is_empty())
                },
            );
            let disclosure = role.eq_ignore_ascii_case("tabpanel")
                || Dom::attribute(dom, node, "hidden") == Some("until-found")
                || (!matches!(role, "dialog" | "alertdialog" | "menu")
                    && Dom::attribute(dom, node, "id").is_some_and(|id| disclosures.contains(id)));
            let aria_hidden = Dom::attribute(dom, node, "aria-hidden")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
            // Angular/Vue remove cloak attributes only after the bound view
            // is ready. If one remains at dump time, its template text is not
            // reader content even when a missing stylesheet fails to hide it.
            let uninitialized_template = ["ng-cloak", "data-ng-cloak", "x-ng-cloak", "v-cloak"]
                .iter()
                .any(|name| Dom::attribute(dom, node, name).is_some());
            let far_offscreen = values
                .get(3)
                .is_some_and(|position| matches!(position.as_str(), "absolute" | "fixed"))
                && values
                    .get(4)
                    .and_then(|value| px(value))
                    .is_some_and(|left| left <= -1000.0)
                && values
                    .get(5)
                    .and_then(|value| px(value))
                    .is_some_and(|top| top <= -1000.0);
            let tracking_pixel = matches!(Dom::node_kind(dom, node), NodeKind::Element("img"))
                && Dom::attribute(dom, node, "alt").is_none_or(|alt| alt.trim().is_empty())
                && values
                    .get(8)
                    .and_then(|value| px(value))
                    .is_some_and(|width| width <= 1.5)
                && values
                    .get(9)
                    .and_then(|value| px(value))
                    .is_some_and(|height| height <= 1.5);
            let foreground = match Dom::node_kind(dom, node) {
                NodeKind::Element("font") => Dom::attribute(dom, node, "color")
                    .and_then(css_color)
                    .or_else(|| values.get(10).and_then(|value| css_color(value))),
                _ => values.get(10).and_then(|value| css_color(value)),
            };
            let zero_contrast_leaf = !has_element_child(dom, node)
                && foreground
                    .filter(|color| color.3 > 0.99)
                    .zip(effective_backgrounds.get(&node).copied())
                    .is_some_and(|(foreground, background)| {
                        background.3 > 0.99
                            && (foreground.0, foreground.1, foreground.2)
                                == (background.0, background.1, background.2)
                    });
            let document_root = Some(node) == document_element || Some(node) == body;
            let animated = values
                .get(12)
                .is_some_and(|name| !name.is_empty() && name != "none");
            if ((values.first().is_some_and(|value| value == "none")
                || values
                    .get(2)
                    .is_some_and(|value| value.parse::<f32>() == Ok(0.0))
                    && !lazy_media
                    && !animated
                    && !document_root)
                && !disclosure)
                || aria_hidden
                || uninitialized_template
                || far_offscreen
                || tracking_pixel
                || zero_contrast_leaf
                || dom
                    .parent_node(node)
                    .is_some_and(|parent| suppressed.contains(&parent))
            {
                suppressed.insert(node);
            }
            if values.first().is_some_and(|value| {
                matches!(
                    value.as_str(),
                    "block" | "flow-root" | "flex" | "grid" | "list-item" | "table"
                )
            }) {
                blocks.insert(node);
            } else if values.first().is_some_and(|value| {
                matches!(
                    value.as_str(),
                    "inline-block" | "inline-flex" | "inline-grid" | "inline-table"
                )
            }) {
                inline_boundaries.insert(node);
            }
            if matches!(Dom::node_kind(dom, node), NodeKind::Element("span")) {
                let relative = values.get(3).is_some_and(|position| position == "relative");
                let vertical_align = values.get(7).map(String::as_str).unwrap_or_default();
                if vertical_align == "super"
                    || (relative
                        && values
                            .get(5)
                            .and_then(|value| px(value))
                            .is_some_and(|top| top < -1.0))
                {
                    superscripts.insert(node);
                } else if vertical_align == "sub"
                    || (relative
                        && values
                            .get(6)
                            .and_then(|value| px(value))
                            .is_some_and(|bottom| bottom < -1.0))
                {
                    subscripts.insert(node);
                }
            }
            if let Some(base_url) = base_url {
                let names: &[&str] = match Dom::node_kind(dom, node) {
                    NodeKind::Element("a") => &["href"],
                    NodeKind::Element("img") => {
                        &["src", "data-original", "data-src", "data-lazy-src"]
                    }
                    NodeKind::Element("iframe" | "audio" | "source") => {
                        &["src", "data-src", "data-lazy-src"]
                    }
                    NodeKind::Element("video") => &["src", "poster", "data-src", "data-lazy-src"],
                    _ => &[],
                };
                for &name in names {
                    let Some(value) = Dom::attribute(dom, node, name)
                        .map(str::trim)
                        .filter(|value| !value.is_empty() && !value.starts_with('#'))
                    else {
                        continue;
                    };
                    if let Ok(url) = base_url.join(value) {
                        resolved_urls
                            .entry(node)
                            .or_insert_with(HashMap::new)
                            .insert(name.to_owned(), url.to_string());
                    }
                }
            }
            if values
                .get(1)
                .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse"))
                && !disclosure
                && !document_visibility_is_hidden
            {
                invisible.insert(node);
            }
        }
        Self {
            dom,
            suppressed,
            invisible,
            blocks,
            inline_boundaries,
            superscripts,
            subscripts,
            resolved_urls,
            fragment_links,
        }
    }

    fn next_retained(&self, mut node: Option<NativeNodeId>) -> Option<NativeNodeId> {
        while let Some(id) = node {
            if !self.suppressed.contains(&id) {
                return Some(id);
            }
            node = self.dom.next_sibling(id);
        }
        None
    }
}

fn px(value: &str) -> Option<f32> {
    value.strip_suffix("px")?.trim().parse().ok()
}

fn css_color(value: &str) -> Option<(u8, u8, u8, f32)> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "transparent" => return Some((0, 0, 0, 0.0)),
        "white" => return Some((255, 255, 255, 1.0)),
        "black" => return Some((0, 0, 0, 1.0)),
        _ => {}
    }
    let (body, alpha) = match (
        value
            .strip_prefix("rgba(")
            .and_then(|body| body.strip_suffix(')')),
        value
            .strip_prefix("rgb(")
            .and_then(|body| body.strip_suffix(')')),
    ) {
        (Some(body), _) => (body, true),
        (_, Some(body)) => (body, false),
        _ => return None,
    };
    let mut parts = body.split(',').map(str::trim);
    let red = parts.next()?.parse().ok()?;
    let green = parts.next()?.parse().ok()?;
    let blue = parts.next()?.parse().ok()?;
    let opacity = if alpha {
        parts.next()?.parse().ok()?
    } else {
        1.0
    };
    parts
        .next()
        .is_none()
        .then_some((red, green, blue, opacity))
}

fn has_element_child(dom: &NativeDom, node: NativeNodeId) -> bool {
    let mut child = dom.first_child(node);
    while let Some(id) = child {
        if matches!(Dom::node_kind(dom, id), NodeKind::Element(_)) {
            return true;
        }
        child = dom.next_sibling(id);
    }
    false
}

fn within(dom: &NativeDom, mut node: NativeNodeId, ancestor: NativeNodeId) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        let Some(parent) = dom.parent_node(node) else {
            return false;
        };
        node = parent;
    }
}

fn text(dom: &NativeDom, root: NativeNodeId, skip: Option<NativeNodeId>) -> String {
    let mut pending = vec![root];
    let mut result = String::new();
    while let Some(node) = pending.pop() {
        if Some(node) == skip {
            continue;
        }
        match Dom::node_kind(dom, node) {
            NodeKind::Text(value) => result.push_str(value),
            NodeKind::Element("script" | "style" | "template") => continue,
            _ => {}
        }
        let mut children = Vec::new();
        let mut child = dom.first_child(node);
        while let Some(id) = child {
            children.push(id);
            child = dom.next_sibling(id);
        }
        pending.extend(children.into_iter().rev());
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Dom for MarkdownDom<'_> {
    type NodeId = NativeNodeId;

    fn node_kind(&self, node: Self::NodeId) -> NodeKind<'_> {
        if self.suppressed.contains(&node) {
            return NodeKind::Other;
        }
        let kind = Dom::node_kind(self.dom, node);
        if self.invisible.contains(&node) {
            // Visibility can be overridden by a descendant. Keep traversing
            // without emitting this element's own image, link or formatting.
            return NodeKind::Document;
        }
        if matches!(kind, NodeKind::Text(_))
            && self
                .dom
                .parent_node(node)
                .is_some_and(|parent| self.invisible.contains(&parent))
        {
            return NodeKind::Other;
        }
        if self.superscripts.contains(&node) {
            return NodeKind::Element("sup");
        }
        if self.subscripts.contains(&node) {
            return NodeKind::Element("sub");
        }
        kind
    }

    fn first_child(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.next_retained(self.dom.first_child(node))
    }

    fn next_sibling(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.next_retained(self.dom.next_sibling(node))
    }

    fn attribute(&self, node: Self::NodeId, name: &str) -> Option<&str> {
        self.resolved_urls
            .get(&node)
            .and_then(|attributes| attributes.get(name))
            .map(String::as_str)
            .or_else(|| Dom::attribute(self.dom, node, name))
    }

    fn has_block_layout(&self, node: Self::NodeId) -> bool {
        self.blocks.contains(&node)
    }

    fn has_text_boundary(&self, node: Self::NodeId) -> bool {
        self.inline_boundaries.contains(&node)
    }

    fn may_have_fragment_links(&self) -> bool {
        self.fragment_links
    }
}
