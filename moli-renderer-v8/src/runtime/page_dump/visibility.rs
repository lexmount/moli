use std::collections::{HashMap, HashSet};

use moli_html2md::{Dom, NodeKind};
use url::Url;

use crate::dom::forms::InputType;
use crate::dom::native::{NativeDom, NativeNodeId};

/// Computed properties needed for document export. Paint effects such as
/// opacity and composited colors do not determine whether source text exists.
#[derive(Clone, Default)]
pub(super) struct ExportStyle {
    pub display: String,
    pub original_display: String,
    pub position: String,
    pub float: String,
    pub flex_direction: String,
    pub visibility: String,
    pub baseline_shift: String,
    pub own_visibility: bool,
}

/// Read the current page's computed visibility without mutating the live DOM.
/// Explicit disclosure content remains useful in a document dump, even while
/// its tab or section is closed. Other non-rendered UI is not document text.
pub(super) struct MarkdownDom<'a> {
    dom: &'a NativeDom,
    frame_roots: HashMap<NativeNodeId, NativeNodeId>,
    suppressed: HashSet<NativeNodeId>,
    invisible: HashSet<NativeNodeId>,
    blocks: HashSet<NativeNodeId>,
    inline_boundaries: HashSet<NativeNodeId>,
    superscripts: HashSet<NativeNodeId>,
    subscripts: HashSet<NativeNodeId>,
    attribute_overrides: HashMap<NativeNodeId, HashMap<String, String>>,
    unchecked_controls: HashSet<NativeNodeId>,
    hidden_frame_owners: HashSet<NativeNodeId>,
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
        document_bases: HashMap<NativeNodeId, Url>,
        frame_roots: HashMap<NativeNodeId, NativeNodeId>,
    ) -> Self {
        // The caller supplies element styles in DOM pre-order. Inherited state
        // below is computed once, after the parent has been visited.
        let styles: Vec<_> = styles.into_iter().collect();
        let owner_document = |node_id: NativeNodeId| {
            dom.node(node_id).and_then(|node| {
                if node.is_document() {
                    Some(node_id)
                } else {
                    node.owner_document()
                }
            })
        };
        let documents: HashSet<_> = styles
            .iter()
            .filter_map(|(node, _)| owner_document(*node))
            .collect();
        let mut document_roots = HashSet::new();
        for document in &documents {
            document_roots.extend(dom.document_element_handle_for_document(*document));
            document_roots.extend(dom.document_body_handle_for_document(*document));
        }
        let hidden_documents: HashSet<_> = styles
            .iter()
            .filter_map(|(node, values)| {
                (document_roots.contains(node)
                    && matches!(values.visibility.as_str(), "hidden" | "collapse"))
                .then(|| owner_document(*node))
                .flatten()
            })
            .collect();
        let root_document = dom.document_node_id();
        let fragment_links = styles.iter().any(|(node, _)| {
            owner_document(*node) == Some(root_document)
                && Dom::attribute(dom, *node, "href").is_some_and(|href| {
                    href.strip_prefix('#')
                        .is_some_and(|fragment| !fragment.is_empty())
                })
        });
        let mut disclosure_roots = HashSet::new();
        let mut targets = HashMap::new();
        for (node, _) in &styles {
            if let (Some(document), Some(id)) =
                (owner_document(*node), Dom::attribute(dom, *node, "id"))
            {
                targets.entry((document, id)).or_insert(*node);
            }
        }
        for (node, _) in &styles {
            if (Dom::attribute(dom, *node, "aria-expanded").is_some()
                || Dom::attribute(dom, *node, "role").is_some_and(|role| has_role(role, "tab")))
                && let Some(document) = owner_document(*node)
                && let Some(controlled) = Dom::attribute(dom, *node, "aria-controls")
            {
                disclosure_roots.extend(controlled.split_ascii_whitespace().filter_map(|id| {
                    let target = targets.get(&(document, id)).copied()?;
                    target_is_disclosure(dom, target).then_some(target)
                }));
            }
        }
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
        let mut hidden_frame_owners = HashSet::new();
        let mut visibility_modes = HashMap::new();
        let mut source_regions = HashSet::new();
        // Flex/grid item flow is determined by the parent, while absolute and
        // floating descendants do not introduce document paragraphs.
        let item_blocks: HashMap<_, _> = styles
            .iter()
            .filter_map(|(node, values)| {
                let blocks = match values.display.as_str() {
                    "flex" | "inline-flex" => values.flex_direction.starts_with("column"),
                    "grid" => true,
                    "inline-grid" => false,
                    _ => return None,
                };
                Some((*node, blocks))
            })
            .collect();
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
                let authored_control =
                    matches!(element.input_type(), InputType::Range | InputType::Color);
                if (element.is_html_input() && !authored_control) || element.input_value_dirty() {
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
            let document = owner_document(node);
            let document_root = document_roots.contains(&node);
            let parent_visibility = dom
                .parent_node(node)
                .and_then(|parent| visibility_modes.get(&parent).copied())
                .unwrap_or(VisibilityMode::Normal);
            let computed_visibility = values.visibility.as_str();
            let visibility_mode = if in_source {
                VisibilityMode::Normal
            } else if document_root
                && document.is_some_and(|document| hidden_documents.contains(&document))
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
                    NodeKind::Element("tr" | "thead" | "tbody" | "tfoot")
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
            let display = if values.original_display.is_empty() {
                values.display.as_str()
            } else {
                values.original_display.as_str()
            };
            let positioned = matches!(values.position.as_str(), "absolute" | "fixed");
            let parent_item = dom
                .parent_node(node)
                .and_then(|parent| item_blocks.get(&parent).copied());
            let out_of_flow = positioned
                || (parent_item.is_none() && !matches!(values.float.as_str(), "" | "none"));
            let item_block = if positioned { None } else { parent_item };
            if let Some(block) = item_block {
                if block {
                    blocks.insert(node);
                } else {
                    inline_boundaries.insert(node);
                }
            } else if !out_of_flow
                && !matches!(display, "" | "none" | "contents" | "inline" | "inline flow")
            {
                if display.starts_with("inline") {
                    // Atomic inline glyph wrappers do not invent word spaces.
                    if matches!(
                        Dom::node_kind(dom, node),
                        NodeKind::Element("input" | "textarea" | "select" | "button")
                    ) || display.contains("list-item")
                    {
                        inline_boundaries.insert(node);
                    }
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
            if let Some(base_url) = document.and_then(|document| document_bases.get(&document)) {
                let tag = match Dom::node_kind(dom, node) {
                    NodeKind::Element(tag) => tag,
                    _ => "",
                };
                for name in moli_html2md::resource_url_attributes(tag) {
                    let Some(value) =
                        Dom::attribute(dom, node, name)
                            .map(str::trim)
                            .filter(|value| {
                                !value.is_empty()
                                    && (!value.starts_with('#')
                                        || document != Some(dom.document_node_id()))
                            })
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
                if frame_roots.contains_key(&node) {
                    hidden_frame_owners.insert(node);
                }
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
            frame_roots,
            suppressed,
            invisible,
            blocks,
            inline_boundaries,
            superscripts,
            subscripts,
            attribute_overrides,
            unchecked_controls,
            hidden_frame_owners,
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

fn target_is_disclosure(dom: &NativeDom, target: NativeNodeId) -> bool {
    content_context(dom, target)
        && (matches!(
            Dom::node_kind(dom, target),
            NodeKind::Element("section" | "article" | "details")
        ) || Dom::attribute(dom, target, "role").is_some_and(|role| {
            ["tabpanel", "region"]
                .iter()
                .any(|value| has_role(role, value))
        }))
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
        if self.frame_roots.contains_key(&node)
            && matches!(kind, NodeKind::Element("iframe" | "frame"))
        {
            return NodeKind::Element("div");
        }
        kind
    }

    fn first_child(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        if self.hidden_frame_owners.contains(&node) {
            return None;
        }
        if let Some(root) = self.frame_roots.get(&node).copied() {
            return self.next_retained(Some(root));
        }
        self.next_retained(self.dom.first_child(node))
    }

    fn next_sibling(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.next_retained(self.dom.next_sibling(node))
    }

    fn attribute(&self, node: Self::NodeId, name: &str) -> Option<&str> {
        if matches!(name, "id" | "name")
            && self
                .dom
                .node(node)
                .and_then(|node| node.owner_document())
                .is_some_and(|document| document != self.dom.document_node_id())
        {
            return None;
        }
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
