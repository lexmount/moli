use std::collections::{HashMap, HashSet};

use moli_html2md::{Dom, NodeKind};
use url::Url;

use crate::dom::native::{NativeDom, NativeNodeId};

/// Computed properties needed for document export. Paint effects such as
/// opacity and composited colors do not determine whether source text exists.
#[derive(Default)]
pub(super) struct ExportStyle {
    pub display: String,
    pub visibility: String,
    pub baseline_shift: String,
    pub own_visibility: bool,
}

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
    attribute_overrides: HashMap<NativeNodeId, HashMap<String, String>>,
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
        styles: impl IntoIterator<Item = (NativeNodeId, ExportStyle)>,
        base_url: Option<&Url>,
    ) -> Self {
        // The caller supplies element styles in DOM pre-order. Inherited state
        // below is computed once, after the parent has been visited.
        let styles: Vec<_> = styles.into_iter().collect();
        let document_element = dom.document_element_node_id();
        let body = dom.body_node_id();
        let document_visibility_is_hidden = styles.iter().any(|(node, values)| {
            (Some(*node) == document_element || Some(*node) == body)
                && matches!(values.visibility.as_str(), "hidden" | "collapse")
        });
        let fragment_links = styles.iter().any(|(node, _)| {
            Dom::attribute(dom, *node, "href").is_some_and(|href| {
                href.strip_prefix('#')
                    .is_some_and(|fragment| !fragment.is_empty())
            })
        });
        let mut disclosures = HashSet::new();
        let mut targets = HashMap::new();
        for (node, _) in &styles {
            if let Some(id) = Dom::attribute(dom, *node, "id") {
                targets.entry(id).or_insert(*node);
            }
        }
        for (node, _) in &styles {
            if (Dom::attribute(dom, *node, "aria-expanded").is_some()
                || Dom::attribute(dom, *node, "role").is_some_and(|role| has_role(role, "tab")))
                && let Some(controlled) = Dom::attribute(dom, *node, "aria-controls")
            {
                disclosures.extend(
                    controlled
                        .split_ascii_whitespace()
                        .filter(|id| target_is_disclosure(dom, &targets, id)),
                );
            }
        }
        let mut disclosure_roots: HashSet<_> = disclosures
            .iter()
            .filter_map(|id| targets.get(id).copied())
            .collect();
        disclosure_roots.extend(styles.iter().filter_map(|(node, _)| {
            let panel =
                Dom::attribute(dom, *node, "role").is_some_and(|role| has_role(role, "tabpanel"));
            let searchable = Dom::attribute(dom, *node, "hidden")
                .is_some_and(|value| value.eq_ignore_ascii_case("until-found"));
            ((panel || searchable) && content_context(dom, *node)).then_some(*node)
        }));
        let hidden_nodes: HashSet<_> = styles
            .iter()
            .filter_map(|(node, values)| (values.display == "none").then_some(*node))
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
        let mut suppressed = HashSet::new();
        let mut invisible = HashSet::new();
        let mut blocks = HashSet::new();
        let mut inline_boundaries = HashSet::new();
        let mut superscripts = HashSet::new();
        let mut subscripts = HashSet::new();
        let mut attribute_overrides = HashMap::new();
        let mut unchecked_controls = HashSet::new();
        let mut visibility_modes = HashMap::new();
        let mut source_regions = HashSet::new();
        let node_order: Vec<_> = styles.iter().map(|(node, _)| *node).collect();
        for (node, values) in styles {
            debug_assert!(
                dom.parent_node(node).is_none_or(|parent| {
                    !matches!(Dom::node_kind(dom, parent), NodeKind::Element(_))
                        || visibility_modes.contains_key(&parent)
                }),
                "document export requires parent-before-child styles"
            );
            // A visible preformatted block is source text. Collapsible line spans
            // belong to that source; hidden outer blocks remain excluded.
            let in_source = dom
                .parent_node(node)
                .is_some_and(|parent| source_regions.contains(&parent));

            if matches!(
                Dom::node_kind(dom, node),
                NodeKind::Element("input" | "textarea")
            ) && let Some(element) = dom.node(node).and_then(|node| node.as_element())
            {
                // A clean textarea gets its default from child text, not the
                // control-state slot (which is initially empty). Once dirty,
                // even an empty live value is authoritative.
                let synthetic_default = Dom::attribute(dom, node, "type").is_some_and(|kind| {
                    kind.eq_ignore_ascii_case("range") || kind.eq_ignore_ascii_case("color")
                }) && Dom::attribute(dom, node, "value").is_none();
                if (element.is_html_input() && !synthetic_default) || element.input_value_dirty() {
                    attribute_overrides
                        .entry(node)
                        .or_insert_with(HashMap::new)
                        .insert("value".to_owned(), element.input_value());
                }
                if matches!(Dom::node_kind(dom, node), NodeKind::Element("input"))
                    && element.checked()
                {
                    attribute_overrides
                        .entry(node)
                        .or_insert_with(HashMap::new)
                        .insert("checked".to_owned(), String::new());
                } else if matches!(Dom::node_kind(dom, node), NodeKind::Element("input")) {
                    unchecked_controls.insert(node);
                }
            }
            let disclosure_root = disclosure_roots.contains(&node);
            let disclosure_path = disclosure_paths.contains(&node);
            let parent_is_disclosure_path = dom
                .parent_node(node)
                .is_some_and(|parent| disclosure_paths.contains(&parent));
            if disclosure_path {
                invisible.insert(node);
            }
            let document_root = Some(node) == document_element || Some(node) == body;
            let parent_visibility = dom
                .parent_node(node)
                .and_then(|parent| visibility_modes.get(&parent).copied())
                .unwrap_or(VisibilityMode::Normal);
            let computed_visibility = values.visibility.as_str();
            let visibility_mode = if in_source {
                VisibilityMode::Normal
            } else if document_root
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
                    VisibilityMode::RootVeil if !values.own_visibility => VisibilityMode::RootVeil,
                    VisibilityMode::DisclosureVeil if !values.own_visibility => {
                        VisibilityMode::DisclosureVeil
                    }
                    _ => VisibilityMode::Hidden,
                }
            } else {
                parent_visibility
            };
            visibility_modes.insert(node, visibility_mode);
            let collapsed_table_part = values.visibility == "collapse"
                && matches!(
                    Dom::node_kind(dom, node),
                    NodeKind::Element("tr" | "thead" | "tbody" | "tfoot" | "col" | "colgroup")
                );
            if collapsed_table_part
                || (values.display == "none" && !in_source && !disclosure_root && !disclosure_path)
                || (parent_is_disclosure_path && !disclosure_root && !disclosure_path)
                || dom
                    .parent_node(node)
                    .is_some_and(|parent| suppressed.contains(&parent))
                    && !disclosure_root
                    && !disclosure_path
            {
                suppressed.insert(node);
            }
            if !suppressed.contains(&node)
                && visibility_mode != VisibilityMode::Hidden
                && (in_source || matches!(Dom::node_kind(dom, node), NodeKind::Element("pre")))
            {
                source_regions.insert(node);
            }
            let display = values.display.as_str();
            if !matches!(display, "" | "none" | "contents" | "inline" | "inline flow") {
                if display.starts_with("inline") {
                    inline_boundaries.insert(node);
                } else {
                    blocks.insert(node);
                }
            }
            if matches!(Dom::node_kind(dom, node), NodeKind::Element("span")) {
                let vertical_align = values.baseline_shift.as_str();
                if vertical_align == "super" {
                    superscripts.insert(node);
                } else if vertical_align == "sub" {
                    subscripts.insert(node);
                }
            }
            if let Some(base_url) = base_url {
                let tag = match Dom::node_kind(dom, node) {
                    NodeKind::Element(tag) => tag,
                    _ => "",
                };
                for name in moli_html2md::resource_url_attributes(tag) {
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
                        attribute_overrides
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
        // Invisible cells still occupy columns. A wholly invisible table/list
        // has no structure to export, unless a descendant restores visibility.
        let mut visible_subtrees = HashSet::new();
        for node in node_order.into_iter().rev() {
            if suppressed.contains(&node) {
                continue;
            }
            if !invisible.contains(&node) || visible_subtrees.contains(&node) {
                if let Some(parent) = dom.parent_node(node) {
                    visible_subtrees.insert(parent);
                }
            } else if matches!(
                Dom::node_kind(dom, node),
                NodeKind::Element("table" | "ul" | "ol" | "dl")
            ) {
                suppressed.insert(node);
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
            attribute_overrides,
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

fn content_context(dom: &NativeDom, mut node: NativeNodeId) -> bool {
    loop {
        if matches!(
            Dom::node_kind(dom, node),
            NodeKind::Element("nav" | "header" | "footer" | "dialog")
        ) || Dom::attribute(dom, node, "popover").is_some()
            || Dom::attribute(dom, node, "role").is_some_and(|role| {
                [
                    "navigation",
                    "banner",
                    "contentinfo",
                    "menu",
                    "dialog",
                    "alertdialog",
                ]
                .iter()
                .any(|excluded| has_role(role, excluded))
            })
        {
            return false;
        }
        let Some(parent) = dom.parent_node(node) else {
            return true;
        };
        node = parent;
    }
}

fn target_is_disclosure(dom: &NativeDom, targets: &HashMap<&str, NativeNodeId>, id: &str) -> bool {
    targets.get(id).is_some_and(|&target| {
        content_context(dom, target)
            && (matches!(
                Dom::node_kind(dom, target),
                NodeKind::Element("section" | "article" | "details")
            ) || Dom::attribute(dom, target, "role").is_some_and(|role| {
                ["tabpanel", "region"]
                    .iter()
                    .any(|value| has_role(role, value))
            }))
    })
}

fn has_role(value: &str, expected: &str) -> bool {
    value
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case(expected))
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

impl Dom for MarkdownDom<'_> {
    type NodeId = NativeNodeId;

    fn node_kind(&self, node: Self::NodeId) -> NodeKind<'_> {
        if self.suppressed.contains(&node) {
            return NodeKind::Other;
        }
        let kind = Dom::node_kind(self.dom, node);
        if self.invisible.contains(&node)
            && !matches!(
                kind,
                NodeKind::Element(
                    "table"
                        | "thead"
                        | "tbody"
                        | "tfoot"
                        | "tr"
                        | "td"
                        | "th"
                        | "caption"
                        | "ul"
                        | "ol"
                        | "li"
                        | "dl"
                        | "dt"
                        | "dd"
                )
            )
        {
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
        self.attribute_overrides
            .get(&node)
            .and_then(|attributes| attributes.get(name))
            .map(String::as_str)
            .or_else(|| Dom::attribute(self.dom, node, name))
    }

    fn control_value(&self, node: Self::NodeId) -> Option<&str> {
        self.attribute_overrides
            .get(&node)
            .and_then(|attributes| attributes.get("value"))
            .map(String::as_str)
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
