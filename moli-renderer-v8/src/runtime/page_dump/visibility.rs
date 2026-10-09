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
    unchecked_controls: HashSet<NativeNodeId>,
    fragment_links: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisibilityMode {
    Normal,
    RootVeil,
    DisclosureVeil,
    Hidden,
}

impl<'a> MarkdownDom<'a> {
    pub(super) fn new(
        dom: &'a NativeDom,
        styles: impl IntoIterator<Item = (NativeNodeId, Vec<String>)>,
        base_url: Option<&Url>,
        final_opacity_elements: HashSet<NativeNodeId>,
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
        let mut non_solid_backgrounds = HashSet::new();
        if styles
            .iter()
            .any(|(_, values)| values.get(11).is_some_and(|value| !value.is_empty()))
        {
            for (node, values) in &styles {
                let own_color = values.get(11).and_then(|value| css_color(value));
                let own = own_color.filter(|color| color.3 > 0.99);
                let inherited = dom
                    .parent_node(*node)
                    .and_then(|parent| effective_backgrounds.get(&parent).copied());
                effective_backgrounds
                    .insert(*node, own.or(inherited).unwrap_or((255, 255, 255, 1.0)));
                let own_image = values
                    .get(13)
                    .is_some_and(|image| !image.is_empty() && image != "none");
                let inherited_image = own.is_none()
                    && dom
                        .parent_node(*node)
                        .is_some_and(|parent| non_solid_backgrounds.contains(&parent));
                let translucent = own_color.is_some_and(|color| color.3 > 0.0 && color.3 <= 0.99);
                if own_image || inherited_image || translucent {
                    non_solid_backgrounds.insert(*node);
                }
            }
        }
        let mut disclosures = HashSet::new();
        let mut excerpts = HashSet::new();
        let targets: HashMap<_, _> = styles
            .iter()
            .filter_map(|(node, _)| Dom::attribute(dom, *node, "id").map(|id| (id, *node)))
            .collect();
        let hidden_targets: HashMap<_, _> = styles
            .iter()
            .filter_map(|(node, values)| {
                values
                    .first()
                    .is_some_and(|value| value == "none")
                    .then(|| Dom::attribute(dom, *node, "id").map(|id| (id, *node)))
                    .flatten()
            })
            .collect();
        for (node, _) in &styles {
            if (Dom::attribute(dom, *node, "aria-expanded").is_some()
                || Dom::attribute(dom, *node, "role")
                    .is_some_and(|role| role.eq_ignore_ascii_case("tab")))
                && let Some(controlled) = Dom::attribute(dom, *node, "aria-controls")
            {
                disclosures.extend(
                    controlled
                        .split_ascii_whitespace()
                        .filter(|id| target_is_disclosure(dom, &targets, id)),
                );
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
                let Some(&target) = hidden_targets.get(id) else {
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
                        if let Some(container) = dom.parent_node(paragraph)
                            && within(dom, target, container)
                            && !within(dom, target, paragraph)
                        {
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
                        break;
                    }
                    ancestor = dom.parent_node(paragraph);
                }
            }
        }
        let disclosure_roots: HashSet<_> = disclosures
            .iter()
            .filter_map(|id| targets.get(id).copied())
            .collect();
        let hidden_nodes: HashSet<_> = styles
            .iter()
            .filter_map(|(node, values)| {
                (values.first().is_some_and(|value| value == "none")
                    || values
                        .get(1)
                        .is_some_and(|value| matches!(value.as_str(), "hidden" | "collapse")))
                .then_some(*node)
            })
            .collect();
        let mut disclosure_paths = HashSet::new();
        for &target in &disclosure_roots {
            let mut chain = Vec::new();
            let mut current = dom.parent_node(target);
            while let Some(node) = current {
                chain.push(node);
                current = dom.parent_node(node);
            }
            if let Some(last_hidden) = chain.iter().rposition(|node| hidden_nodes.contains(node)) {
                disclosure_paths.extend(chain.into_iter().take(last_hidden + 1));
            }
        }
        let mut suppressed = excerpts;
        let mut invisible = HashSet::new();
        let mut blocks = HashSet::new();
        let mut inline_boundaries = HashSet::new();
        let mut superscripts = HashSet::new();
        let mut subscripts = HashSet::new();
        let mut resolved_urls = HashMap::new();
        let mut unchecked_controls = HashSet::new();
        let mut visibility_modes = HashMap::new();
        for (node, values) in styles {
            let role = Dom::attribute(dom, node, "role").unwrap_or_default();
            if matches!(
                Dom::node_kind(dom, node),
                NodeKind::Element("input" | "textarea")
            ) && let Some(element) = dom.node(node).and_then(|node| node.as_element())
            {
                resolved_urls
                    .entry(node)
                    .or_insert_with(HashMap::new)
                    .insert("value".to_owned(), element.input_value());
                if matches!(Dom::node_kind(dom, node), NodeKind::Element("input"))
                    && element.checked()
                {
                    resolved_urls
                        .entry(node)
                        .or_insert_with(HashMap::new)
                        .insert("checked".to_owned(), String::new());
                } else if matches!(Dom::node_kind(dom, node), NodeKind::Element("input")) {
                    unchecked_controls.insert(node);
                }
            }
            // Lazy media often stays transparent until a scroll/load event.
            // Its declared resource still belongs to the document, unlike
            // transparent numeric placeholders or hidden UI text.
            let lazy_media = matches!(
                Dom::node_kind(dom, node),
                NodeKind::Element("img" | "iframe" | "video" | "audio")
            ) && ["data-original", "data-src", "data-lazy-src", "data-srcset"]
                .iter()
                .any(|attribute| {
                    Dom::attribute(dom, node, attribute)
                        .is_some_and(|value| !value.trim().is_empty())
                });
            let disclosure_root = disclosure_roots.contains(&node)
                || role.eq_ignore_ascii_case("tabpanel")
                || Dom::attribute(dom, node, "hidden") == Some("until-found")
                || (!matches!(role, "dialog" | "alertdialog" | "menu")
                    && Dom::attribute(dom, node, "id").is_some_and(|id| disclosures.contains(id)));
            let disclosure_path = disclosure_paths.contains(&node);
            let parent_is_disclosure_path = dom
                .parent_node(node)
                .is_some_and(|parent| disclosure_paths.contains(&parent));
            if disclosure_path {
                invisible.insert(node);
            }
            // Angular/Vue remove cloak attributes only after the bound view
            // is ready. If one remains at dump time, its template text is not
            // reader content even when a missing stylesheet fails to hide it.
            let uninitialized_template = ["ng-cloak", "data-ng-cloak", "x-ng-cloak", "v-cloak"]
                .iter()
                .any(|name| Dom::attribute(dom, node, name).is_some());
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
            let foreground = values.get(10).and_then(|value| css_color(value));
            let zero_contrast_leaf = !has_element_child(dom, node)
                && has_text_child(dom, node)
                && !non_solid_backgrounds.contains(&node)
                && values
                    .get(12)
                    .is_none_or(|shadow| shadow.is_empty() || shadow == "none")
                && foreground
                    .filter(|color| color.3 > 0.99)
                    .zip(effective_backgrounds.get(&node).copied())
                    .is_some_and(|(foreground, background)| {
                        background.3 > 0.99
                            && (foreground.0, foreground.1, foreground.2)
                                == (background.0, background.1, background.2)
                    });
            let document_root = Some(node) == document_element || Some(node) == body;
            let parent_visibility = dom
                .parent_node(node)
                .and_then(|parent| visibility_modes.get(&parent).copied())
                .unwrap_or(VisibilityMode::Normal);
            let computed_visibility = values.get(1).map(String::as_str).unwrap_or("visible");
            let visibility_mode = if document_root
                && document_visibility_is_hidden
                && matches!(computed_visibility, "hidden" | "collapse")
            {
                VisibilityMode::RootVeil
            } else if disclosure_root && matches!(computed_visibility, "hidden" | "collapse") {
                VisibilityMode::DisclosureVeil
            } else if computed_visibility == "visible" {
                VisibilityMode::Normal
            } else if matches!(computed_visibility, "hidden" | "collapse") {
                match parent_visibility {
                    VisibilityMode::RootVeil
                        if values.get(14).is_none_or(|value| value != "true") =>
                    {
                        VisibilityMode::RootVeil
                    }
                    VisibilityMode::DisclosureVeil
                        if values.get(14).is_none_or(|value| value != "true") =>
                    {
                        VisibilityMode::DisclosureVeil
                    }
                    _ => VisibilityMode::Hidden,
                }
            } else {
                parent_visibility
            };
            visibility_modes.insert(node, visibility_mode);
            if ((values.first().is_some_and(|value| value == "none")
                || values
                    .get(2)
                    .is_some_and(|value| value.parse::<f32>() == Ok(0.0))
                    && !lazy_media
                    && !final_opacity_elements.contains(&node)
                    && !document_root)
                && !disclosure_root
                && !disclosure_path)
                || uninitialized_template
                || tracking_pixel
                || (zero_contrast_leaf && !disclosure_root && !disclosure_path)
                || (parent_is_disclosure_path && !disclosure_root && !disclosure_path)
                || dom
                    .parent_node(node)
                    .is_some_and(|parent| suppressed.contains(&parent))
                    && !disclosure_root
                    && !disclosure_path
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
                let vertical_align = values.get(7).map(String::as_str).unwrap_or_default();
                if vertical_align == "super" {
                    superscripts.insert(node);
                } else if vertical_align == "sub" {
                    subscripts.insert(node);
                }
            }
            if let Some(base_url) = base_url {
                let names: &[&str] = match Dom::node_kind(dom, node) {
                    NodeKind::Element("a") => &["href"],
                    NodeKind::Element("img") => &[
                        "src",
                        "data-original",
                        "data-src",
                        "data-lazy-src",
                        "srcset",
                        "data-srcset",
                    ],
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
                    let resolved = if matches!(name, "srcset" | "data-srcset") {
                        resolve_srcset(base_url, value)
                    } else {
                        base_url.join(value).ok().map(|url| url.to_string())
                    };
                    if let Some(url) = resolved {
                        resolved_urls
                            .entry(node)
                            .or_insert_with(HashMap::new)
                            .insert(name.to_owned(), url);
                    }
                }
            }
            if visibility_mode == VisibilityMode::Hidden && !disclosure_root && !disclosure_path {
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
            unchecked_controls,
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

fn target_is_disclosure<D: Dom + ?Sized>(
    dom: &D,
    targets: &HashMap<&str, D::NodeId>,
    id: &str,
) -> bool {
    targets.get(id).is_some_and(|target| {
        !Dom::attribute(dom, *target, "role").is_some_and(|role| {
            ["dialog", "alertdialog", "menu"]
                .iter()
                .any(|excluded| role.eq_ignore_ascii_case(excluded))
        })
    })
}

fn px(value: &str) -> Option<f32> {
    value.strip_suffix("px")?.trim().parse().ok()
}

fn resolve_srcset(base_url: &Url, value: &str) -> Option<String> {
    let resolved: Vec<_> = moli_html2md::parse_srcset(value)
        .into_iter()
        .filter_map(|candidate| {
            let url = base_url.join(candidate.url).ok()?;
            Some(if candidate.descriptor.is_empty() {
                url.to_string()
            } else {
                format!("{url} {}", candidate.descriptor)
            })
        })
        .collect();
    (!resolved.is_empty()).then(|| resolved.join(", "))
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

fn has_text_child(dom: &NativeDom, node: NativeNodeId) -> bool {
    let mut child = dom.first_child(node);
    while let Some(id) = child {
        if matches!(Dom::node_kind(dom, id), NodeKind::Text(text) if !text.trim().is_empty()) {
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
        if name == "checked" && self.unchecked_controls.contains(&node) {
            return None;
        }
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
