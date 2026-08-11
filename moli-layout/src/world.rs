use std::{collections::HashMap, fmt::Debug, hash::Hash, sync::Arc};

use style::Atom;
use taffy::{Cache, Layout, Point, Style};

use crate::{
    LayoutElementSemantics, LayoutError, LayoutPoint, LayoutPseudo, ReplacedMetrics,
    ResolvedLayoutStyle, inline::InlineFormattingContext,
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
    IntrinsicSizingKeywordDeferred,
    GridTemplateModeDeferred,
    GeneratedContentUnsupported,
}

/// Hypothetical position contributed by an out-of-flow placeholder in one IFC.
///
/// Coordinates are relative to the formatting-context owner's border box.
/// The record exists only for the current layout pass and is consumed before
/// Taffy's rounding traversal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineStaticPosition {
    pub(crate) owner: LayoutBoxId,
    pub(crate) point: Point<f32>,
    pub(crate) inline_level: bool,
}

impl LayoutCapabilityDiagnostic {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ListMarkerStyleFallback => "list-marker-style-fallback",
            Self::TextProjectionDeferred => "text-projection-deferred",
            Self::PositionedStaticPositionDeferred => "positioned-static-position-deferred",
            Self::IntrinsicSizingKeywordDeferred => "intrinsic-sizing-keyword-deferred",
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
    /// Static position emitted by the shared IFC's out-of-flow placeholder.
    pub(crate) inline_static_position: Option<InlineStaticPosition>,
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
    pub(crate) replaced_metrics: Option<ReplacedMetrics>,
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
    pub(crate) viewport_layout: ViewportLayoutState,
}

impl<N> LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) fn new(root: LayoutBox<N>) -> Self {
        Self {
            boxes: vec![root],
            source_mapping: HashMap::new(),
            display_contents_mapping: HashMap::new(),
            root: LayoutBoxId::from_index(0),
            viewport_layout: ViewportLayoutState::default(),
        }
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
            child_box.parent = Some(parent);
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
        replaced_metrics: Option<ReplacedMetrics>,
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
            parent: None,
            children: Vec::new(),
            layout_parent: None,
            layout_children: Vec::new(),
            positioned_containing_block: None,
            inline_static_position: None,
            style,
            text,
            text_selection: None,
            inline_layout: None,
            inline_context_owner: None,
            inline_flattened: false,
            outside_list_marker: false,
            scroll_offset: LayoutPoint::ZERO,
            replaced_metrics,
            replaced_image: None,
            css_images: crate::source::LayoutCssImageResources::default(),
            collapsed_table_borders: None,
            collapsed_table_border_part: false,
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
    if style.has_deferred_intrinsic_sizing() {
        push_diagnostic(
            &mut diagnostics,
            LayoutCapabilityDiagnostic::IntrinsicSizingKeywordDeferred,
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
