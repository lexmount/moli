use std::{collections::HashMap, fmt::Debug, hash::Hash, sync::Arc};

use style::Atom;
use taffy::{Cache, Dimension, Layout, LayoutEnvironment, LogicalStaticPosition, Style};

use crate::{
    LayoutElementSemantics, LayoutError, LayoutPoint, LayoutPseudo, ResolvedLayoutStyle,
    inline::InlineFormattingContext, replaced::ReplacedContext,
};

/// Dense identifier scoped to exactly one [`LayoutWorld`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutBoxId(u32);

impl LayoutBoxId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("one layout pass exceeded the u32 box limit"))
    }

    pub(crate) fn to_taffy(self) -> taffy::NodeId {
        taffy::NodeId::from(self.index())
    }

    pub(crate) fn from_taffy(node: taffy::NodeId) -> Self {
        Self::from_index(usize::from(node))
    }
}

/// Browser-level box role used by construction, diagnostics, and dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutBoxKind {
    PrincipalBlock,
    PrincipalFlowRoot,
    PrincipalFlex,
    PrincipalInlineFlex,
    PrincipalGrid,
    PrincipalInlineGrid,
    PrincipalInline,
    PrincipalInlineBlock,
    ListItem,
    InlineListItem,
    TableWrapper,
    InlineTableWrapper,
    TableCaption,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableColumnGroup,
    TableColumn,
    TableRow,
    TableCell,
    FormControl,
    LineBreak,
    Replaced,
    /// Content-bearing fallback layout object for a terminally unavailable
    /// image that HTML asks the user agent to treat as a sized atomic object.
    ImageFallback,
    AnonymousBlock,
    AnonymousFlexItem,
    AnonymousGridItem,
    AnonymousTableWrapper,
    AnonymousTableRowGroup,
    AnonymousTableRow,
    AnonymousTableCell,
    InlineContinuation,
    Text,
    PseudoMarker,
    PseudoBefore,
    PseudoAfter,
}

impl LayoutBoxKind {
    pub(crate) const fn is_text(self) -> bool {
        matches!(self, Self::Text)
    }

    pub(crate) const fn debug_name(self) -> &'static str {
        match self {
            Self::PrincipalBlock => "principal-block",
            Self::PrincipalFlowRoot => "principal-flow-root",
            Self::PrincipalFlex => "principal-flex",
            Self::PrincipalInlineFlex => "principal-inline-flex",
            Self::PrincipalGrid => "principal-grid",
            Self::PrincipalInlineGrid => "principal-inline-grid",
            Self::PrincipalInline => "principal-inline",
            Self::PrincipalInlineBlock => "principal-inline-block",
            Self::ListItem => "list-item",
            Self::InlineListItem => "inline-list-item",
            Self::TableWrapper => "table-wrapper",
            Self::InlineTableWrapper => "inline-table-wrapper",
            Self::TableCaption => "table-caption",
            Self::TableRowGroup => "table-row-group",
            Self::TableHeaderGroup => "table-header-group",
            Self::TableFooterGroup => "table-footer-group",
            Self::TableColumnGroup => "table-column-group",
            Self::TableColumn => "table-column",
            Self::TableRow => "table-row",
            Self::TableCell => "table-cell",
            Self::FormControl => "form-control",
            Self::LineBreak => "line-break",
            Self::Replaced => "replaced",
            Self::ImageFallback => "image-fallback",
            Self::AnonymousBlock => "anonymous-block",
            Self::AnonymousFlexItem => "anonymous-flex-item",
            Self::AnonymousGridItem => "anonymous-grid-item",
            Self::AnonymousTableWrapper => "anonymous-table-wrapper",
            Self::AnonymousTableRowGroup => "anonymous-table-row-group",
            Self::AnonymousTableRow => "anonymous-table-row",
            Self::AnonymousTableCell => "anonymous-table-cell",
            Self::InlineContinuation => "inline-continuation",
            Self::Text => "text",
            Self::PseudoMarker => "pseudo-marker",
            Self::PseudoBefore => "pseudo-before",
            Self::PseudoAfter => "pseudo-after",
        }
    }
}

/// Why a box with no DOM source was introduced by box construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutAnonymousReason {
    MixedFlowInlineRun,
    InlineSplitContinuation,
    FlexTextRun,
    GridTextRun,
    MissingTableParent,
    MissingTableRowGroup,
    MissingTableRow,
    MissingTableCell,
    FormControlContent,
}

impl LayoutAnonymousReason {
    pub(crate) const fn debug_name(self) -> &'static str {
        match self {
            Self::MixedFlowInlineRun => "mixed-flow-inline-run",
            Self::InlineSplitContinuation => "inline-split-continuation",
            Self::FlexTextRun => "flex-text-run",
            Self::GridTextRun => "grid-text-run",
            Self::MissingTableParent => "missing-table-parent",
            Self::MissingTableRowGroup => "missing-table-row-group",
            Self::MissingTableRow => "missing-table-row",
            Self::MissingTableCell => "missing-table-cell",
            Self::FormControlContent => "form-control-content",
        }
    }
}

/// Stable indication that construction succeeded but a later numeric/paint phase is deferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutCapabilityDiagnostic {
    ListMarkerStyleFallback,
    TextProjectionDeferred,
    PositionedStaticPositionDeferred,
    AnchorSizingDeferred,
    GridTemplateModeDeferred,
    GeneratedContentUnsupported,
}

/// Static-position candidate retained between its original formatting context
/// and the numeric ancestor that supplies the actual containing block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OutOfFlowStaticPosition {
    pub(crate) owner: LayoutBoxId,
    pub(crate) position: LogicalStaticPosition,
}

/// Real positioned child exposed to its original formatting context while its
/// numeric layout parent remains the actual containing block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutOfFlowCandidateChild {
    pub(crate) child: LayoutBoxId,
    pub(crate) insertion_index: usize,
}

impl LayoutCapabilityDiagnostic {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ListMarkerStyleFallback => "list-marker-style-fallback",
            Self::TextProjectionDeferred => "text-projection-deferred",
            Self::PositionedStaticPositionDeferred => "positioned-static-position-deferred",
            Self::AnchorSizingDeferred => "anchor-sizing-deferred",
            Self::GridTemplateModeDeferred => "grid-template-mode-deferred",
            Self::GeneratedContentUnsupported => "generated-content-unsupported",
        }
    }
}

/// One box in the pass-local CSS box tree.
#[derive(Debug)]
pub struct LayoutBox<N> {
    pub(crate) source: Option<N>,
    pub(crate) owner: Option<N>,
    pub(crate) pseudo: Option<LayoutPseudo>,
    pub(crate) source_label: String,
    pub(crate) owner_label: Option<String>,
    pub(crate) element_semantics: Option<LayoutElementSemantics>,
    pub(crate) anonymous_reason: Option<LayoutAnonymousReason>,
    pub(crate) capability_diagnostics: Vec<LayoutCapabilityDiagnostic>,
    pub(crate) kind: LayoutBoxKind,
    /// Parent in the source-backed LayoutObject hierarchy, before anonymous
    /// box normalization and block-in-inline promotion.
    ///
    /// Chromium keeps this ancestry on its LayoutObject tree while fragment
    /// construction handles block-in-inline placement. Moli's normalized
    /// `parent` tree is intentionally different, so containing-block and
    /// CSSOM ancestry retain their own first-class relation here.
    pub(crate) structural_parent: Option<LayoutBoxId>,
    /// Parent in the normalized formatting tree, including anonymous wrappers
    /// and block-in-inline promotion.
    pub(crate) parent: Option<LayoutBoxId>,
    pub(crate) children: Vec<LayoutBoxId>,
    /// Parent used by the numeric layout algorithm.
    ///
    /// This differs from `parent` for absolute/fixed boxes whose containing
    /// block is not their direct box-tree parent.
    pub(crate) layout_parent: Option<LayoutBoxId>,
    pub(crate) layout_children: Vec<LayoutBoxId>,
    /// CSS containing block selected from the construction tree.
    ///
    /// This can differ from `layout_parent` when an absolutely positioned box
    /// is contained by a flattened inline box. `layout_parent` remains a real
    /// numeric-tree node; this field retains the semantic containing block.
    pub(crate) positioned_containing_block: Option<LayoutBoxId>,
    /// Static position emitted by the original formatting context.
    pub(crate) out_of_flow_static_position: Option<OutOfFlowStaticPosition>,
    /// Positioned children whose static position this formatting context owns
    /// even though their numeric parent is an ancestor containing block.
    pub(crate) out_of_flow_candidates: Vec<OutOfFlowCandidateChild>,
    pub(crate) style: ResolvedLayoutStyle,
    pub(crate) text: Option<Arc<str>>,
    pub(crate) text_selection: Option<crate::LayoutTextSelection>,
    /// Shared text/inline layout owned by this formatting-context root.
    pub(crate) inline_layout: Option<InlineFormattingContext>,
    /// Outermost IFC that consumes this box as a flattened or atomic item.
    pub(crate) inline_context_owner: Option<LayoutBoxId>,
    /// Text, `<br>`, and non-atomic inline boxes are laid out by their owner IFC
    /// and therefore do not enter Taffy's child traversal independently.
    pub(crate) inline_flattened: bool,
    /// `list-style-position: outside` marker laid out beside, rather than in,
    /// the list item's principal inline formatting context.
    pub(crate) outside_list_marker: bool,
    /// Current source-owned scroll offset sampled at construction time.
    pub(crate) scroll_offset: LayoutPoint,
    /// Pass-local natural sizing retained once at box construction.
    pub(crate) replaced_context: Option<ReplacedContext>,
    pub(crate) replaced_image: Option<crate::LayoutImageResource>,
    pub(crate) css_images: crate::source::LayoutCssImageResources,
    /// Winning collapsed-table edges owned by the table wrapper for this pass.
    ///
    /// The record contains only resolved numeric/color/style data. It is
    /// produced before Taffy sizing, completed with grid-line geometry after
    /// sizing, and consumed once by immutable paint projection.
    pub(crate) collapsed_table_borders: Option<crate::table::CollapsedTableBorders>,
    /// Suppresses ordinary per-box border paint for parts participating in a
    /// collapsed table. Their authored borders have already entered the table
    /// owner's conflict-resolution grid.
    pub(crate) collapsed_table_border_part: bool,
    /// Authored logical `min-inline-size` saved while the parent-facing Taffy
    /// style projects the table's GRID_MIN as `min-content`.
    ///
    /// Blink treats a table's intrinsic grid minimum as an additional lower
    /// bound after authored min/max constraints. Taffy's generic block model
    /// has only one `min-size` slot, so the outer tree exposes `min-content`
    /// there and the table formatter retains the authored value here.
    pub(crate) table_authored_min_inline_size: Option<Dimension>,
    pub(crate) inline_formatting_context: bool,
    pub(crate) cache: Cache,
    pub(crate) unrounded_layout: Layout,
    pub(crate) final_layout: Layout,
}

/// Layout-only initial containing block.
///
/// The CSS viewport is not a DOM box, but Taffy needs a real root node so the
/// root element can remain auto-height while fixed and root-level absolute
/// boxes resolve against the viewport rather than the root element's box.
#[derive(Debug)]
pub(crate) struct ViewportLayoutState {
    pub(crate) children: Vec<LayoutBoxId>,
    pub(crate) style: Style<Atom>,
    pub(crate) cache: Cache,
    pub(crate) unrounded_layout: Layout,
    pub(crate) final_layout: Layout,
}

impl Default for ViewportLayoutState {
    fn default() -> Self {
        Self {
            children: Vec::new(),
            style: Style::default(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            final_layout: Layout::with_order(0),
        }
    }
}

impl<N> LayoutBox<N> {
    pub fn kind(&self) -> LayoutBoxKind {
        self.kind
    }

    pub fn source(&self) -> Option<N>
    where
        N: Copy,
    {
        self.source
    }

    pub fn owner(&self) -> Option<N>
    where
        N: Copy,
    {
        self.owner
    }

    pub fn pseudo(&self) -> Option<LayoutPseudo> {
        self.pseudo
    }

    pub fn element_semantics(&self) -> Option<&LayoutElementSemantics> {
        self.element_semantics.as_ref()
    }

    pub fn anonymous_reason(&self) -> Option<LayoutAnonymousReason> {
        self.anonymous_reason
    }

    pub fn capability_diagnostics(&self) -> &[LayoutCapabilityDiagnostic] {
        &self.capability_diagnostics
    }

    pub fn source_label(&self) -> &str {
        &self.source_label
    }

    pub fn children(&self) -> &[LayoutBoxId] {
        &self.children
    }

    /// Returns the parent in the source-backed LayoutObject hierarchy.
    pub fn structural_parent(&self) -> Option<LayoutBoxId> {
        self.structural_parent
    }

    pub fn style(&self) -> &ResolvedLayoutStyle {
        &self.style
    }

    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn establishes_inline_formatting_context(&self) -> bool {
        self.inline_formatting_context
    }

    pub fn final_layout(&self) -> Layout {
        self.final_layout
    }

    pub(crate) fn is_replaced(&self) -> bool {
        self.element_semantics
            .as_ref()
            .is_some_and(LayoutElementSemantics::is_replaced)
    }

    /// Resolve the used ratio at the layout-node boundary, after both authored
    /// style and natural replaced-element sizing are available.
    pub(crate) fn resolved_aspect_ratio(&self) -> taffy::ResolvedAspectRatio {
        // Blink drops a replaced element's natural ratio when any applicable
        // size containment is active. An explicit authored ratio still wins,
        // and `auto <ratio>` can still use its authored fallback.
        let natural_ratio = (!self.applies_any_size_containment())
            .then(|| {
                self.replaced_context
                    .and_then(|context| context.inherent_ratio())
            })
            .flatten();
        self.style.resolved_aspect_ratio(natural_ratio)
    }
}

/// Entire short-lived sidecar used for one construction/layout demand.
#[derive(Debug)]
pub struct LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) boxes: Vec<LayoutBox<N>>,
    pub(crate) source_mapping: HashMap<N, LayoutBoxId>,
    pub(crate) display_contents_mapping: HashMap<N, Vec<LayoutBoxId>>,
    pub(crate) root: LayoutBoxId,
    pub(crate) document_element: N,
    pub(crate) document_body: Option<N>,
    pub(crate) document_mode: crate::LayoutDocumentMode,
    pub(crate) viewport_scroll_offset: crate::LayoutPoint,
    pub(crate) viewport_layout: ViewportLayoutState,
    /// Document/view state shared by the active layout pass.
    pub(crate) layout_environment: LayoutEnvironment,
}

impl<N> LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) fn new(
        root: LayoutBox<N>,
        document_element: N,
        document_body: Option<N>,
        document_mode: crate::LayoutDocumentMode,
        viewport_scroll_offset: crate::LayoutPoint,
    ) -> Self {
        Self {
            boxes: vec![root],
            source_mapping: HashMap::new(),
            display_contents_mapping: HashMap::new(),
            root: LayoutBoxId::from_index(0),
            document_element,
            document_body,
            document_mode,
            viewport_scroll_offset,
            viewport_layout: ViewportLayoutState::default(),
            layout_environment: LayoutEnvironment::NONE,
        }
    }

    pub(crate) fn is_document_element(&self, id: LayoutBoxId) -> bool {
        id == self.root
    }

    pub(crate) fn is_document_body(&self, id: LayoutBoxId) -> bool {
        self.document_body
            .is_some_and(|body| self.boxes[id.index()].source == Some(body))
    }

    /// Whether this box's overflow is propagated to the layout viewport.
    ///
    /// The root always propagates. In an HTML document the body also
    /// propagates while the root's computed overflow remains visible. This is
    /// the LayoutObject-level distinction Blink exposes through
    /// `IsScrollContainer()`: computed overflow remains authored, but the box
    /// no longer owns a local scrolling mechanism.
    pub(crate) fn overflow_propagates_to_viewport(&self, id: LayoutBoxId) -> bool {
        self.is_document_element(id)
            || (self.is_document_body(id) && !self.boxes[self.root.index()].style.clips_overflow())
    }

    pub(crate) fn clips_overflow(&self, id: LayoutBoxId) -> bool {
        !self.overflow_propagates_to_viewport(id) && self.boxes[id.index()].style.clips_overflow()
    }

    pub(crate) fn establishes_scroll_container(&self, id: LayoutBoxId) -> bool {
        !self.overflow_propagates_to_viewport(id)
            && self.boxes[id.index()].style.establishes_scroll_container()
    }

    pub(crate) fn clips_descendant_paint(&self, id: LayoutBoxId) -> bool {
        let layout_box = &self.boxes[id.index()];
        self.clips_overflow(id)
            || (layout_box.is_eligible_for_paint_or_layout_containment()
                && layout_box.style.applies_paint_containment())
    }

    pub(crate) fn document_scrolling_element(&self) -> Option<N> {
        if self.document_mode != crate::LayoutDocumentMode::Quirks {
            return Some(self.document_element);
        }
        let body = self.document_body?;
        let body_is_scroll_container = self
            .source_mapping
            .get(&body)
            .is_some_and(|id| self.establishes_scroll_container(*id));
        (!body_is_scroll_container).then_some(body)
    }

    /// Whether this box participates in the HTML body-fills-viewport quirk.
    ///
    /// The available size remains constraint-space state. This predicate only
    /// identifies the two eligible layout objects, matching Blink's
    /// `BlockNode::IsQuirkyAndFillsViewport` exclusions.
    pub(crate) fn is_quirky_viewport_filler(&self, id: LayoutBoxId) -> bool {
        let layout_box = &self.boxes[id.index()];
        if self.document_mode != crate::LayoutDocumentMode::Quirks
            || layout_box.style.is_absolute_positioned()
            || layout_box.style.is_fixed_positioned()
            || layout_box.style.is_floated()
            || layout_box.style.display().is_inline_level()
        {
            return false;
        }
        self.is_document_element(id) || self.is_document_body(id)
    }

    pub fn root(&self) -> LayoutBoxId {
        self.root
    }

    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    pub fn box_by_id(&self, id: LayoutBoxId) -> Option<&LayoutBox<N>> {
        self.boxes.get(id.index())
    }

    pub fn source_box(&self, source: N) -> Option<LayoutBoxId> {
        self.source_mapping.get(&source).copied()
    }

    pub(crate) fn box_by_id_mut(&mut self, id: LayoutBoxId) -> Option<&mut LayoutBox<N>> {
        self.boxes.get_mut(id.index())
    }

    pub(crate) fn viewport_taffy_node(&self) -> taffy::NodeId {
        taffy::NodeId::from(self.boxes.len())
    }

    pub(crate) fn is_viewport_taffy_node(&self, node: taffy::NodeId) -> bool {
        usize::from(node) == self.boxes.len()
    }

    pub(crate) fn global_layout_origin(&self, id: LayoutBoxId) -> (f32, f32) {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut current = Some(id);
        while let Some(box_id) = current {
            let layout_box = &self.boxes[box_id.index()];
            x += layout_box.final_layout.location.x;
            y += layout_box.final_layout.location.y;
            current = layout_box.layout_parent;
        }
        (x, y)
    }

    pub(crate) fn allocate(&mut self, layout_box: LayoutBox<N>) -> LayoutBoxId {
        let id = LayoutBoxId::from_index(self.boxes.len());
        self.boxes.push(layout_box);
        id
    }

    pub(crate) fn map_source(&mut self, source: N, id: LayoutBoxId) {
        self.source_mapping.entry(source).or_insert(id);
    }

    pub(crate) fn map_display_contents_source(&mut self, source: N, child_boxes: &[LayoutBoxId]) {
        self.display_contents_mapping
            .entry(source)
            .or_default()
            .extend_from_slice(child_boxes);
    }

    pub(crate) fn replace_children(
        &mut self,
        parent: LayoutBoxId,
        children: Vec<LayoutBoxId>,
    ) -> Result<(), LayoutError> {
        let Some(parent_box) = self.box_by_id_mut(parent) else {
            return Err(LayoutError::InvalidBoxReference {
                index: parent.index(),
            });
        };
        parent_box.children = children.clone();
        for child in children {
            let Some(child_box) = self.box_by_id_mut(child) else {
                return Err(LayoutError::InvalidBoxReference {
                    index: child.index(),
                });
            };
            // Raw source ownership is recorded before normalization. Boxes
            // synthesized by normalization have no earlier owner, so their
            // first formatting attachment is also their structural parent.
            child_box.structural_parent.get_or_insert(parent);
            child_box.parent = Some(parent);
        }
        Ok(())
    }

    /// Attaches one newly synthesized box through the same ownership seam as
    /// construction-time children.
    pub(crate) fn append_synthesized_child(
        &mut self,
        parent: LayoutBoxId,
        child: LayoutBoxId,
    ) -> Result<(), LayoutError> {
        let Some(parent_box) = self.box_by_id(parent) else {
            return Err(LayoutError::InvalidBoxReference {
                index: parent.index(),
            });
        };
        let mut children = parent_box.children.clone();
        children.push(child);
        self.replace_children(parent, children)
    }

    /// Records source/LayoutObject ownership before formatting normalization
    /// is allowed to wrap or promote any child.
    ///
    /// The first owner deliberately wins. A split inline returns its promoted
    /// block in an ancestor's child stream later, but that reattachment must
    /// not erase the inline LayoutObject that originally owned the block.
    pub(crate) fn record_structural_children(
        &mut self,
        parent: LayoutBoxId,
        children: &[LayoutBoxId],
    ) -> Result<(), LayoutError> {
        if self.box_by_id(parent).is_none() {
            return Err(LayoutError::InvalidBoxReference {
                index: parent.index(),
            });
        }
        for child in children {
            let Some(child_box) = self.box_by_id_mut(*child) else {
                return Err(LayoutError::InvalidBoxReference {
                    index: child.index(),
                });
            };
            child_box.structural_parent.get_or_insert(parent);
        }
        Ok(())
    }

    pub(crate) fn compact_reachable(&mut self) {
        let mut reachable = vec![false; self.boxes.len()];
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            if std::mem::replace(&mut reachable[id.index()], true) {
                continue;
            }
            stack.extend(self.boxes[id.index()].children.iter().copied());
        }

        let mut remap = vec![None; self.boxes.len()];
        let mut next_index = 0;
        for (index, is_reachable) in reachable.iter().copied().enumerate() {
            if is_reachable {
                remap[index] = Some(LayoutBoxId::from_index(next_index));
                next_index += 1;
            }
        }

        let mut compacted = Vec::with_capacity(next_index);
        for (index, mut layout_box) in self.boxes.drain(..).enumerate() {
            if !reachable[index] {
                continue;
            }
            layout_box.structural_parent = layout_box
                .structural_parent
                .and_then(|parent| remap[parent.index()]);
            layout_box.parent = layout_box.parent.and_then(|parent| remap[parent.index()]);
            layout_box.children = layout_box
                .children
                .into_iter()
                .filter_map(|child| remap[child.index()])
                .collect();
            compacted.push(layout_box);
        }
        self.boxes = compacted;
        self.root = remap[self.root.index()].expect("the layout root is always reachable");
        self.source_mapping.retain(|_, id| {
            let Some(remapped) = remap[id.index()] else {
                return false;
            };
            *id = remapped;
            true
        });
        self.display_contents_mapping.retain(|_, ids| {
            *ids = ids.iter().filter_map(|id| remap[id.index()]).collect();
            !ids.is_empty()
        });
    }

    /// Validates graph ownership invariants without relying on allocator IDs.
    pub fn validate_invariants(&self) -> Result<(), LayoutError> {
        let mut reachable = vec![false; self.boxes.len()];
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let Some(layout_box) = self.box_by_id(id) else {
                return Err(LayoutError::InvalidBoxReference { index: id.index() });
            };
            if std::mem::replace(&mut reachable[id.index()], true) {
                return Err(LayoutError::SourceCycle {
                    source_label: layout_box.source_label.clone(),
                });
            }
            for child in layout_box.children.iter().rev().copied() {
                let Some(child_box) = self.box_by_id(child) else {
                    return Err(LayoutError::InvalidBoxReference {
                        index: child.index(),
                    });
                };
                if child_box.parent != Some(id) {
                    return Err(LayoutError::InvalidBoxReference {
                        index: child.index(),
                    });
                }
                stack.push(child);
            }
        }
        if let Some(index) = reachable.iter().position(|reachable| !reachable) {
            return Err(LayoutError::InvalidBoxReference { index });
        }
        for (index, layout_box) in self.boxes.iter().enumerate() {
            let id = LayoutBoxId::from_index(index);
            self.validate_box_provenance(id, layout_box)?;
            self.validate_table_parentage(id, layout_box)?;
        }
        Ok(())
    }

    fn validate_box_provenance(
        &self,
        id: LayoutBoxId,
        layout_box: &LayoutBox<N>,
    ) -> Result<(), LayoutError> {
        if id == self.root {
            if layout_box.structural_parent.is_some() {
                return Err(LayoutError::InvalidBoxReference { index: id.index() });
            }
        } else if layout_box.structural_parent.is_none() {
            return Err(LayoutError::InvalidBoxReference { index: id.index() });
        }
        if let Some(parent) = layout_box.structural_parent
            && (parent == id || self.box_by_id(parent).is_none())
        {
            return Err(LayoutError::InvalidBoxReference {
                index: parent.index(),
            });
        }
        if let Some(source) = layout_box.source {
            if layout_box.owner.is_some()
                || layout_box.pseudo.is_some()
                || layout_box.anonymous_reason.is_some()
            {
                return Err(LayoutError::source_contract(
                    &layout_box.source_label,
                    "a source-backed box cannot also be pseudo/anonymous-owned",
                ));
            }
            if self.source_mapping.get(&source) != Some(&id) {
                return Err(LayoutError::source_contract(
                    &layout_box.source_label,
                    "source mapping does not point to its source-backed box",
                ));
            }
        }
        if layout_box.pseudo.is_some()
            && (layout_box.source.is_some() || layout_box.owner.is_none())
        {
            return Err(LayoutError::source_contract(
                &layout_box.source_label,
                "a pseudo box must have an owner and no DOM source",
            ));
        }
        if layout_box.anonymous_reason.is_some()
            && (layout_box.source.is_some() || layout_box.owner.is_none())
        {
            return Err(LayoutError::source_contract(
                &layout_box.source_label,
                "an anonymous box must have an owner and no DOM source",
            ));
        }
        Ok(())
    }

    fn validate_table_parentage(
        &self,
        id: LayoutBoxId,
        layout_box: &LayoutBox<N>,
    ) -> Result<(), LayoutError> {
        let role = table_role(layout_box.style.display());
        if id != self.root
            && let Some(role) = role
        {
            let parent = layout_box
                .parent
                .and_then(|parent| self.box_by_id(parent))
                .and_then(|parent| table_role(parent.style.display()));
            let valid = match role {
                TableInvariantRole::Root => true,
                TableInvariantRole::Caption
                | TableInvariantRole::RowGroup
                | TableInvariantRole::ColumnGroup => parent == Some(TableInvariantRole::Root),
                TableInvariantRole::Column => matches!(
                    parent,
                    Some(TableInvariantRole::Root | TableInvariantRole::ColumnGroup)
                ),
                TableInvariantRole::Row => parent == Some(TableInvariantRole::RowGroup),
                TableInvariantRole::Cell => parent == Some(TableInvariantRole::Row),
            };
            if !valid {
                return Err(LayoutError::source_contract(
                    &layout_box.source_label,
                    format!(
                        "table role {} has invalid parent role {}",
                        role.debug_name(),
                        parent.map_or("non-table", TableInvariantRole::debug_name)
                    ),
                ));
            }
        }

        let children_are_valid = match role {
            Some(TableInvariantRole::Root) => layout_box.children.iter().all(|child| {
                self.box_by_id(*child)
                    .and_then(|child| table_role(child.style.display()))
                    .is_some_and(TableInvariantRole::is_direct_root_child)
            }),
            Some(TableInvariantRole::RowGroup) => layout_box.children.iter().all(|child| {
                self.box_by_id(*child).is_some_and(|child| {
                    table_role(child.style.display()) == Some(TableInvariantRole::Row)
                })
            }),
            Some(TableInvariantRole::Row) => layout_box.children.iter().all(|child| {
                self.box_by_id(*child).is_some_and(|child| {
                    table_role(child.style.display()) == Some(TableInvariantRole::Cell)
                })
            }),
            Some(TableInvariantRole::ColumnGroup) => layout_box.children.iter().all(|child| {
                self.box_by_id(*child).is_some_and(|child| {
                    table_role(child.style.display()) == Some(TableInvariantRole::Column)
                })
            }),
            Some(TableInvariantRole::Column) => layout_box.children.is_empty(),
            Some(TableInvariantRole::Caption | TableInvariantRole::Cell) | None => true,
        };
        if !children_are_valid {
            return Err(LayoutError::source_contract(
                &layout_box.source_label,
                format!(
                    "table role {} has an invalid direct child",
                    role.expect("only table roles validate table children")
                        .debug_name()
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn new_box(
        source: Option<N>,
        owner: Option<N>,
        pseudo: Option<LayoutPseudo>,
        source_label: String,
        owner_label: Option<String>,
        element_semantics: Option<LayoutElementSemantics>,
        anonymous_reason: Option<LayoutAnonymousReason>,
        kind: LayoutBoxKind,
        style: ResolvedLayoutStyle,
        text: Option<Arc<str>>,
    ) -> LayoutBox<N> {
        let capability_diagnostics =
            default_capability_diagnostics(kind, element_semantics.as_ref(), &style);
        LayoutBox {
            source,
            owner,
            pseudo,
            source_label,
            owner_label,
            element_semantics,
            anonymous_reason,
            capability_diagnostics,
            kind,
            structural_parent: None,
            parent: None,
            children: Vec::new(),
            layout_parent: None,
            layout_children: Vec::new(),
            positioned_containing_block: None,
            out_of_flow_static_position: None,
            out_of_flow_candidates: Vec::new(),
            style,
            text,
            text_selection: None,
            inline_layout: None,
            inline_context_owner: None,
            inline_flattened: false,
            outside_list_marker: false,
            scroll_offset: LayoutPoint::ZERO,
            replaced_context: None,
            replaced_image: None,
            css_images: crate::source::LayoutCssImageResources::default(),
            collapsed_table_borders: None,
            collapsed_table_border_part: false,
            table_authored_min_inline_size: None,
            inline_formatting_context: false,
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            final_layout: Layout::with_order(0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableInvariantRole {
    Root,
    Caption,
    RowGroup,
    ColumnGroup,
    Column,
    Row,
    Cell,
}

impl TableInvariantRole {
    const fn is_direct_root_child(self) -> bool {
        matches!(
            self,
            Self::Caption | Self::RowGroup | Self::ColumnGroup | Self::Column
        )
    }

    const fn debug_name(self) -> &'static str {
        match self {
            Self::Root => "table-root",
            Self::Caption => "table-caption",
            Self::RowGroup => "table-row-group",
            Self::ColumnGroup => "table-column-group",
            Self::Column => "table-column",
            Self::Row => "table-row",
            Self::Cell => "table-cell",
        }
    }
}

const fn table_role(display: crate::LayoutDisplay) -> Option<TableInvariantRole> {
    use crate::LayoutDisplay as Display;
    match display {
        Display::Table | Display::InlineTable => Some(TableInvariantRole::Root),
        Display::TableCaption => Some(TableInvariantRole::Caption),
        Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup => {
            Some(TableInvariantRole::RowGroup)
        }
        Display::TableColumnGroup => Some(TableInvariantRole::ColumnGroup),
        Display::TableColumn => Some(TableInvariantRole::Column),
        Display::TableRow => Some(TableInvariantRole::Row),
        Display::TableCell => Some(TableInvariantRole::Cell),
        Display::None
        | Display::Contents
        | Display::Block
        | Display::FlowRoot
        | Display::Inline
        | Display::InlineBlock
        | Display::Flex
        | Display::InlineFlex
        | Display::Grid
        | Display::InlineGrid
        | Display::BlockListItem
        | Display::InlineListItem => None,
    }
}

fn default_capability_diagnostics(
    kind: LayoutBoxKind,
    _semantics: Option<&LayoutElementSemantics>,
    style: &ResolvedLayoutStyle,
) -> Vec<LayoutCapabilityDiagnostic> {
    use LayoutBoxKind as Kind;
    let mut diagnostics = Vec::new();
    let kind_diagnostic = match kind {
        Kind::TableWrapper
        | Kind::InlineTableWrapper
        | Kind::TableCaption
        | Kind::TableRowGroup
        | Kind::TableHeaderGroup
        | Kind::TableFooterGroup
        | Kind::TableColumnGroup
        | Kind::TableColumn
        | Kind::TableRow
        | Kind::TableCell
        | Kind::AnonymousTableWrapper
        | Kind::AnonymousTableRowGroup
        | Kind::AnonymousTableRow
        | Kind::AnonymousTableCell
        | Kind::ListItem
        | Kind::InlineListItem
        | Kind::PseudoMarker
        | Kind::FormControl => None,
        Kind::LineBreak => None,
        Kind::Replaced => None,
        Kind::ImageFallback => None,
        Kind::PrincipalBlock
        | Kind::PrincipalFlowRoot
        | Kind::PrincipalFlex
        | Kind::PrincipalInlineFlex
        | Kind::PrincipalGrid
        | Kind::PrincipalInlineGrid
        | Kind::PrincipalInline
        | Kind::PrincipalInlineBlock
        | Kind::AnonymousBlock
        | Kind::AnonymousFlexItem
        | Kind::AnonymousGridItem
        | Kind::InlineContinuation
        | Kind::Text
        | Kind::PseudoBefore
        | Kind::PseudoAfter => None,
    };
    if let Some(diagnostic) = kind_diagnostic {
        push_diagnostic(&mut diagnostics, diagnostic);
    }
    if style.has_deferred_text_projection() {
        push_diagnostic(
            &mut diagnostics,
            LayoutCapabilityDiagnostic::TextProjectionDeferred,
        );
    }
    if style.has_deferred_anchor_sizing() {
        push_diagnostic(
            &mut diagnostics,
            LayoutCapabilityDiagnostic::AnchorSizingDeferred,
        );
    }
    if style.has_deferred_grid_template_mode() {
        push_diagnostic(
            &mut diagnostics,
            LayoutCapabilityDiagnostic::GridTemplateModeDeferred,
        );
    }
    diagnostics
}

fn push_diagnostic(
    diagnostics: &mut Vec<LayoutCapabilityDiagnostic>,
    diagnostic: LayoutCapabilityDiagnostic,
) {
    if !diagnostics.contains(&diagnostic) {
        diagnostics.push(diagnostic);
    }
}
