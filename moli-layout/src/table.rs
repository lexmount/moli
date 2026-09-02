// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The table-as-grid formatter is narrowly adapted from DioxusLabs/blitz
// commit d788124ab881f9bb537cb452ec1d837604a374a8,
// packages/blitz-dom/src/layout/table.rs. Moli keeps the CSS table box
// tree for provenance/paint and uses a pass-local flattened grid view only for
// numeric track sizing. Blitz 5081c658's calc() cell-width pass-through is
// deliberately not adopted: Chromium 147 treats that value as automatic in
// fixed table layout, producing equal 150px tracks in the pinned differential
// fixture instead of Blitz/Taffy's 130px/170px split.

use std::{fmt::Debug, hash::Hash};

use style::Atom;
use taffy::{
    AutoSizeBehavior, AvailableSpace, CacheTree, DetailedGridInfo, Dimension, Display,
    GridAutoFlow, IntrinsicSizeResult, Layout, LayoutGridContainer, LayoutInput, LayoutOutput,
    LayoutPartialTree, Line, LogicalSize, MaybeResolve, NodeId, OrthogonalFallback, Point, Rect,
    RequestedAxis, ResolveOrZero, RunMode, Size, SizingMode, SizingPurpose, Style,
    TraversePartialTree, TraverseTree, WritingMode, compute_grid_layout, style_helpers,
};

use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutWorld,
    style::{LayoutInlineAlignment, resolve_stylo_calc_value},
};

mod collapsed_borders;
mod columns;

pub(crate) use collapsed_borders::CollapsedTableBorders;
use collapsed_borders::{prepare_collapsed_table_borders, set_collapsed_border_geometry};
use columns::{
    AutomaticTableSizingTarget, TABLE_MAX_INLINE_SIZE, TableCellInlineConstraint,
    TableCellSpanConstraint, TableColumnConstraint, TableLayoutMode, apply_cell_constraints,
    compute_grid_inline_min_max, distribute_auto_columns, distribute_fixed_columns,
    fixed_grid_min_inline_size,
};

#[derive(Clone)]
struct TableCell {
    id: LayoutBoxId,
    style: Style<Atom>,
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
}

#[derive(Clone, Copy)]
struct TableRow {
    id: LayoutBoxId,
    group: Option<LayoutBoxId>,
    index: usize,
    track: taffy::TrackSizingFunction,
    percent: Option<f32>,
    is_constrained: bool,
    has_rowspan_start: bool,
}

#[derive(Clone, Copy)]
struct TableSection {
    start_row: usize,
    row_count: usize,
    fixed_block_size: Option<f32>,
    percent: Option<f32>,
    is_constrained: bool,
    is_tbody: bool,
}

#[derive(Clone, Copy)]
struct TableColumn {
    id: LayoutBoxId,
    group: Option<LayoutBoxId>,
    start: usize,
    span: usize,
}

/// Direct table children grouped by their CSS table role.
///
/// A table's first header and footer groups have a visual position independent
/// of tree order. Keeping this grouping as a first-class input ensures row
/// placement, first-row column constraints, and structural box geometry all
/// consume the same section order.
#[derive(Default)]
struct TableGroupedChildren {
    captions: Vec<LayoutBoxId>,
    columns: Vec<LayoutBoxId>,
    header: Option<LayoutBoxId>,
    bodies: Vec<LayoutBoxId>,
    footer: Option<LayoutBoxId>,
}

impl TableGroupedChildren {
    fn collect<N>(world: &LayoutWorld<N>, root: LayoutBoxId) -> Self
    where
        N: Copy + Debug + Eq + Hash,
    {
        let mut grouped = Self::default();
        for child in world.boxes[root.index()].children.iter().copied() {
            match world.boxes[child.index()].kind {
                LayoutBoxKind::TableCaption => grouped.captions.push(child),
                LayoutBoxKind::TableColumnGroup | LayoutBoxKind::TableColumn => {
                    grouped.columns.push(child)
                }
                LayoutBoxKind::TableHeaderGroup => {
                    if grouped.header.is_none() {
                        grouped.header = Some(child);
                    } else {
                        grouped.bodies.push(child);
                    }
                }
                LayoutBoxKind::TableRowGroup
                | LayoutBoxKind::AnonymousTableRowGroup
                | LayoutBoxKind::TableRow
                | LayoutBoxKind::AnonymousTableRow => grouped.bodies.push(child),
                LayoutBoxKind::TableFooterGroup => {
                    if grouped.footer.is_none() {
                        grouped.footer = Some(child);
                    } else {
                        grouped.bodies.push(child);
                    }
                }
                _ => {}
            }
        }
        grouped
    }

    fn sections(&self) -> impl Iterator<Item = LayoutBoxId> + '_ {
        self.header
            .iter()
            .copied()
            .chain(self.bodies.iter().copied())
            .chain(self.footer.iter().copied())
    }
}

struct TableContext {
    style: Style<Atom>,
    cells: Vec<TableCell>,
    rows: Vec<TableRow>,
    sections: Vec<TableSection>,
    columns: Vec<TableColumn>,
    captions: Vec<LayoutBoxId>,
    detailed: Option<DetailedGridInfo>,
    collapsed_borders: bool,
    column_count: usize,
    column_constraints: Vec<TableColumnConstraint>,
    layout_mode: TableLayoutMode,
    inline_border_spacing: f32,
    block_border_spacing: f32,
    writing_mode: WritingMode,
}

/// Parent-facing min/max-content sizes of the complete table wrapper.
///
/// Column constraints produce GRID_MIN/GRID_MAX. CSS Tables adds one wrapper
/// rule after that calculation: a percentage-dependent fixed table has an
/// effectively unbounded max-content contribution. Keeping the wrapper result
/// separate prevents that rule from contaminating final column distribution.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TableIntrinsicInlineSizes {
    min_content: f32,
    max_content: f32,
}

impl TableIntrinsicInlineSizes {
    fn from_grid(
        grid: columns::TableGridInlineMinMax,
        layout_mode: TableLayoutMode,
        preferred_inline_size: Dimension,
    ) -> Self {
        let max_content =
            if layout_mode.is_fixed() && preferred_inline_size.may_have_percentage_dependence() {
                TABLE_MAX_INLINE_SIZE
            } else {
                grid.max
            };
        Self {
            min_content: grid.min,
            max_content: max_content.max(grid.min),
        }
    }
}

pub(crate) fn prepare_table_layout_trees<N>(world: &mut LayoutWorld<N>)
where
    N: Copy + Debug + Eq + Hash,
{
    let roots = (0..world.boxes.len())
        .map(LayoutBoxId::from_index)
        .filter(|id| is_table_root(world.boxes[id.index()].kind))
        .collect::<Vec<_>>();
    for root in roots {
        let writing_mode = world.boxes[root.index()].style.writing_mode();
        defer_table_block_intrinsics_until_row_layout(
            &mut world.boxes[root.index()].style.taffy,
            writing_mode,
        );
        let mut parts = Vec::new();
        collect_table_parts(world, root, &mut parts);
        if parts.is_empty() {
            continue;
        }
        for layout_box in &mut world.boxes {
            layout_box
                .layout_children
                .retain(|child| !parts.contains(child));
        }
        for part in parts.iter().copied() {
            world.boxes[part.index()].layout_parent = Some(root);
            if is_table_structural(world.boxes[part.index()].kind) {
                world.boxes[part.index()].layout_children.clear();
            }
        }
        world.boxes[root.index()].layout_children.extend(parts);
        prepare_collapsed_table_borders(world, root);
        apply_parent_facing_table_inline_constraints(world, root);
    }
}

/// Keep intrinsic block-axis constraints unresolved until table rows exist.
///
/// A generic formatting context asks its child for a content-based block size
/// before the table adapter has measured row minima. Treating the intermediate
/// Grid result as the table's min/max-content block size can therefore clamp a
/// specified table height to its row content. Blink avoids that ordering bug by
/// passing an indefinite intrinsic block size into `ComputeBlockSizeForFragment`
/// until `TableLayoutAlgorithm::ComputeRows` has completed. Moli expresses the
/// same phase boundary in the pass-local Taffy projection; the authoritative
/// Stylo computed values remain unchanged for CSSOM and subsequent restyles.
fn defer_table_block_intrinsics_until_row_layout(
    style: &mut Style<Atom>,
    writing_mode: WritingMode,
) {
    let defer = |dimension: Dimension| {
        if dimension.is_intrinsic() {
            Dimension::auto()
        } else {
            dimension
        }
    };

    let mut size = writing_mode.to_logical(style.size);
    let mut min_size = writing_mode.to_logical(style.min_size);
    let mut max_size = writing_mode.to_logical(style.max_size);
    size.block_size = defer(size.block_size);
    min_size.block_size = defer(min_size.block_size);
    max_size.block_size = defer(max_size.block_size);
    style.size = writing_mode.to_physical(size);
    style.min_size = writing_mode.to_physical(min_size);
    style.max_size = writing_mode.to_physical(max_size);
}

/// Expose the table grid's minimum inline size to the parent formatting
/// context. The numeric Grid backend only sees the table after its parent has
/// resolved the child's used size, so returning an oversized LayoutOutput is
/// too late to influence that decision.
///
/// Blink performs the equivalent work through `ComputeGridInlineMinMax`
/// before `ComputeUsedInlineSizeForTableFragment`. Moli keeps the same
/// boundary explicit while adapting the table algorithm to Taffy's parent
/// sizing contract.
fn apply_parent_facing_table_inline_constraints<N>(world: &mut LayoutWorld<N>, root: LayoutBoxId)
where
    N: Copy + Debug + Eq + Hash,
{
    let mut context = build_table_context(world, root);
    context.collect_authored_fixed_cell_constraints(world);
    let Some(min_border_box_size) = context.fixed_grid_min_border_box_size() else {
        return;
    };

    let style = &mut world.boxes[root.index()].style.taffy;
    let percentage_basis = None;
    let padding = style
        .padding
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let border = style
        .border
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let parent_inline_insets = padding.left + padding.right + border.left + border.right;
    let min_style_size = if style.box_sizing == taffy::BoxSizing::ContentBox {
        (min_border_box_size - parent_inline_insets).max(0.0)
    } else {
        min_border_box_size
    };

    let current = style.min_size.width;
    if current.is_auto() {
        style.min_size.width = Dimension::length(min_style_size);
    } else if current.tag() == taffy::CompactLength::LENGTH_TAG {
        style.min_size.width = Dimension::length(current.value().max(min_style_size));
    }
}

fn collect_table_parts<N>(world: &LayoutWorld<N>, root: LayoutBoxId, output: &mut Vec<LayoutBoxId>)
where
    N: Copy + Debug + Eq + Hash,
{
    for child in world.boxes[root.index()].children.iter().copied() {
        let kind = world.boxes[child.index()].kind;
        if matches!(
            kind,
            LayoutBoxKind::TableCaption
                | LayoutBoxKind::TableCell
                | LayoutBoxKind::AnonymousTableCell
        ) {
            output.push(child);
            continue;
        }
        if is_table_structural(kind) {
            output.push(child);
            collect_table_parts(world, child, output);
        }
    }
}

pub(crate) fn compute_table_layout<N>(
    world: &mut LayoutWorld<N>,
    root: LayoutBoxId,
    inputs: LayoutInput,
) -> LayoutOutput
where
    N: Copy + Debug + Eq + Hash,
{
    let mut context = build_table_context(world, root);
    context.collect_cell_inline_constraints(world);
    let grid_inputs = context.resolve_column_tracks(inputs);
    let mut output = run_table_grid(world, &mut context, grid_inputs);
    context.encompass_minimum_row_block_size(&mut output);

    if inputs.run_mode == RunMode::PerformLayout && context.resolve_row_tracks(output.size) {
        context.detailed = None;
        output = run_table_grid(world, &mut context, grid_inputs);
        context.encompass_minimum_row_block_size(&mut output);
    }

    if inputs.run_mode == RunMode::PerformLayout {
        let caption_parent_writing_mode = world.boxes[root.index()].style.writing_mode();
        let top_captions = context
            .captions
            .iter()
            .copied()
            .filter(|caption| !world.boxes[caption.index()].style.caption_is_bottom())
            .collect::<Vec<_>>();
        let bottom_captions = context
            .captions
            .iter()
            .copied()
            .filter(|caption| world.boxes[caption.index()].style.caption_is_bottom())
            .collect::<Vec<_>>();
        let top_height = layout_captions(
            world,
            &top_captions,
            output.size.width,
            0.0,
            caption_parent_writing_mode,
        );
        shift_grid_children(world, &context.cells, top_height);
        let bottom_height = layout_captions(
            world,
            &bottom_captions,
            output.size.width,
            top_height + output.size.height,
            caption_parent_writing_mode,
        );
        apply_structural_layout(world, root, &context, top_height, output.size);
        if let Some(first_baseline) = &mut output.first_baselines.y {
            *first_baseline += top_height;
        }
        if let Some(last_baseline) = &mut output.last_baselines.y {
            *last_baseline += top_height;
        }
        output.size.height += top_height + bottom_height;
        output.content_size.height += top_height + bottom_height;
        output.content_size.width = output.content_size.width.min(output.size.width);
        output.content_size.height = output.content_size.height.min(output.size.height);
    }
    output
}

fn run_table_grid<N>(
    world: &mut LayoutWorld<N>,
    context: &mut TableContext,
    inputs: LayoutInput,
) -> LayoutOutput
where
    N: Copy + Debug + Eq + Hash,
{
    let mut wrapper = TableTreeWrapper { world, context };
    compute_grid_layout(&mut wrapper, NodeId::from(0usize), inputs)
}

fn build_table_context<N>(world: &LayoutWorld<N>, root: LayoutBoxId) -> TableContext
where
    N: Copy + Debug + Eq + Hash,
{
    let root_style = &world.boxes[root.index()].style;
    let collapsed = root_style.table_border_is_collapsed();
    let spacing = if collapsed {
        Size::ZERO
    } else {
        root_style.table_border_spacing()
    };
    let mut style = root_style.taffy.clone();
    style.display = Display::Grid;
    style.item_is_table = true;
    // CSS table row distribution is not Grid's align-content: stretch. The
    // table algorithm measures row minima and distributes the table's used
    // block size according to row and section constraints below.
    style.align_content = Some(taffy::AlignContent::START);
    style.grid_auto_flow = GridAutoFlow::RowDense;
    style.grid_auto_columns.clear();
    style.grid_auto_rows.clear();

    let grouped_children = TableGroupedChildren::collect(world, root);
    let mut cells = Vec::new();
    let mut rows = Vec::new();
    let mut sections = Vec::new();
    let mut columns = Vec::new();
    let mut max_columns = 0usize;
    let mut column_tracks = Vec::new();
    let layout_mode = if root_style.uses_fixed_table_layout() {
        TableLayoutMode::Fixed
    } else {
        TableLayoutMode::Automatic
    };
    let writing_mode = root_style.writing_mode();
    for column in grouped_children.columns.iter().copied() {
        collect_columns(world, column, None, &mut columns, &mut column_tracks);
    }
    for section in grouped_children.sections() {
        let start_row = rows.len();
        collect_rows(world, section, None, writing_mode, &mut rows, &mut cells);
        let row_count = rows.len() - start_row;
        if row_count == 0 {
            continue;
        }
        clamp_section_row_percentages(&mut rows[start_row..]);
        let section_kind = world.boxes[section.index()].kind;
        let section_dimension = writing_mode
            .to_logical(world.boxes[section.index()].style.taffy.size)
            .block_size;
        let (fixed_block_size, percent, is_constrained) = table_block_constraint(section_dimension);
        let is_row_group = !matches!(
            section_kind,
            LayoutBoxKind::TableRow | LayoutBoxKind::AnonymousTableRow
        );
        sections.push(TableSection {
            start_row,
            row_count,
            fixed_block_size: is_row_group.then_some(fixed_block_size).flatten(),
            percent: is_row_group.then_some(percent).flatten(),
            is_constrained: is_row_group && is_constrained,
            is_tbody: grouped_children.header != Some(section)
                && grouped_children.footer != Some(section),
        });
    }
    place_table_cells(&mut cells, &rows, &mut max_columns);
    max_columns = max_columns.max(column_tracks.len()).max(1);
    column_tracks.resize(max_columns, TableColumnConstraint::auto());
    for cell in &mut cells {
        cell.style.grid_column = Line {
            start: style_helpers::line((cell.column + 1).min(i16::MAX as usize) as i16),
            end: style_helpers::span(cell.column_span as u16),
        };
        cell.style.grid_row = Line {
            start: style_helpers::line((cell.row + 1).min(i16::MAX as usize) as i16),
            end: style_helpers::span(cell.row_span as u16),
        };
        clear_table_cell_inline_sizing(&mut cell.style, writing_mode);
        normalize_table_cell_block_sizing(&mut cell.style, writing_mode);
    }
    let placeholder_track: taffy::TrackSizingFunction = style_helpers::auto();
    style.grid_template_columns =
        std::iter::repeat_n(placeholder_track.into(), max_columns).collect();
    style.grid_template_rows = if rows.is_empty() {
        vec![style_helpers::auto()]
    } else {
        rows.iter().map(|row| row.track.into()).collect()
    };
    style.gap = Size {
        width: style_helpers::length(spacing.width),
        height: style_helpers::length(spacing.height),
    };
    if !collapsed {
        let padding = style
            .padding
            .resolve_or_zero(None, resolve_stylo_calc_value);
        style.padding = Rect {
            left: style_helpers::length(padding.left + spacing.width),
            right: style_helpers::length(padding.right + spacing.width),
            top: style_helpers::length(padding.top + spacing.height),
            bottom: style_helpers::length(padding.bottom + spacing.height),
        };
    }
    TableContext {
        style,
        cells,
        rows,
        sections,
        columns,
        captions: grouped_children.captions,
        detailed: None,
        collapsed_borders: collapsed,
        column_count: max_columns,
        column_constraints: column_tracks,
        layout_mode,
        inline_border_spacing: spacing.width,
        block_border_spacing: spacing.height,
        writing_mode,
    }
}

impl TableContext {
    /// Gather the fixed-layout first-row widths needed while exposing the
    /// table's parent-facing minimum before an intrinsic measurement pass can
    /// borrow the layout world mutably.
    fn collect_authored_fixed_cell_constraints<N>(&mut self, world: &LayoutWorld<N>)
    where
        N: Copy + Debug + Eq + Hash,
    {
        if !self.layout_mode.is_fixed() {
            return;
        }
        let mut cell_constraints = vec![None; self.column_count];
        let mut cell_spans = Vec::new();
        for cell in self.cells.iter().filter(|cell| cell.row == 0) {
            let constraint = authored_table_cell_inline_constraint(
                &world.boxes[cell.id.index()].style.taffy,
                self.writing_mode,
                self.layout_mode,
            );
            collect_cell_constraint(cell, constraint, &mut cell_constraints, &mut cell_spans);
        }
        apply_cell_constraints(
            &mut self.column_constraints,
            &cell_constraints,
            &mut cell_spans,
            self.inline_border_spacing,
            self.layout_mode,
        );
    }

    /// Gather cell measures after the table tree is complete. Fixed layout
    /// consumes the first visual row; automatic layout consumes every row.
    fn collect_cell_inline_constraints<N>(&mut self, world: &mut LayoutWorld<N>)
    where
        N: Copy + Debug + Eq + Hash,
    {
        let mut cell_constraints = vec![None; self.column_count];
        let mut cell_spans = Vec::new();
        for cell in &self.cells {
            if self.layout_mode.is_fixed() && cell.row != 0 {
                continue;
            }
            let constraint =
                table_cell_inline_constraint(world, cell.id, self.writing_mode, self.layout_mode);
            collect_cell_constraint(cell, constraint, &mut cell_constraints, &mut cell_spans);
        }
        apply_cell_constraints(
            &mut self.column_constraints,
            &cell_constraints,
            &mut cell_spans,
            self.inline_border_spacing,
            self.layout_mode,
        );
    }

    /// Resolve the CSS table's used border-box inline size, synchronize it
    /// with column constraints, and hand only final lengths to Grid.
    fn resolve_column_tracks(&mut self, inputs: LayoutInput) -> LayoutInput {
        let space = inputs.constraint_space(self.writing_mode);
        let percentage_basis = space.margin_padding_percentage_basis();
        let padding = self
            .style
            .padding
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let border = self
            .style
            .border
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let inline_insets = physical_inline_sum(self.writing_mode, padding)
            + physical_inline_sum(self.writing_mode, border);
        let internal_spacing =
            self.inline_border_spacing.max(0.0) * self.column_count.saturating_sub(1) as f32;
        let undistributable_space = inline_insets + internal_spacing;
        let grid_min_max = compute_grid_inline_min_max(
            &self.column_constraints,
            undistributable_space,
            self.layout_mode,
        );
        let preferred_inline_size = self.writing_mode.to_logical(self.style.size).inline_size;
        let intrinsic_inline_sizes = TableIntrinsicInlineSizes::from_grid(
            grid_min_max,
            self.layout_mode,
            preferred_inline_size,
        );
        let used_inline_size = self.resolve_used_inline_size(
            inputs,
            grid_min_max,
            intrinsic_inline_sizes,
            inline_insets,
        );
        let assignable_inline_size = (used_inline_size - undistributable_space).max(0.0);
        let column_sizes = if self.layout_mode.is_fixed() {
            distribute_fixed_columns(assignable_inline_size, &self.column_constraints)
        } else {
            distribute_auto_columns(
                assignable_inline_size,
                &self.column_constraints,
                AutomaticTableSizingTarget::Constrained,
            )
        };
        self.style.grid_template_columns = column_sizes
            .into_iter()
            .map(|size| {
                let track: taffy::TrackSizingFunction = style_helpers::length(size);
                track.into()
            })
            .collect();

        let numeric_inline_size = if self.style.box_sizing == taffy::BoxSizing::ContentBox {
            (used_inline_size - inline_insets).max(0.0)
        } else {
            used_inline_size
        };
        set_physical_inline_dimension(
            self.writing_mode,
            &mut self.style.size,
            style_helpers::length(numeric_inline_size),
        );
        set_physical_inline_dimension(
            self.writing_mode,
            &mut self.style.min_size,
            Dimension::auto(),
        );
        set_physical_inline_dimension(
            self.writing_mode,
            &mut self.style.max_size,
            Dimension::auto(),
        );

        let mut grid_space = space;
        grid_space.known_size.inline_size = Some(used_inline_size);
        grid_space.definite_size.inline_size = Some(used_inline_size);
        grid_space.into_layout_input()
    }

    fn resolve_used_inline_size(
        &self,
        inputs: LayoutInput,
        grid: columns::TableGridInlineMinMax,
        intrinsic: TableIntrinsicInlineSizes,
        inline_insets: f32,
    ) -> f32 {
        let space = inputs.constraint_space(self.writing_mode);
        let (min_content, max_content) =
            if space.sizing_purpose == SizingPurpose::IntrinsicContribution {
                (intrinsic.min_content, intrinsic.max_content)
            } else {
                (grid.min, grid.max)
            };
        let available = space.available_size.inline_size;
        let fit_content = || match available {
            AvailableSpace::Definite(value) => min_content.max(value.max(0.0).min(max_content)),
            AvailableSpace::MinContent => min_content,
            AvailableSpace::MaxContent => max_content,
        };
        let logical_size = self.writing_mode.to_logical(self.style.size);
        let logical_min_size = self.writing_mode.to_logical(self.style.min_size);
        let logical_max_size = self.writing_mode.to_logical(self.style.max_size);
        let percentage_basis = space.percentage_resolution_size.inline_size;
        let box_sizing_adjustment = if self.style.box_sizing == taffy::BoxSizing::ContentBox {
            inline_insets
        } else {
            0.0
        };
        let resolve_dimension = |dimension: Dimension| {
            if dimension.is_min_content() {
                Some(min_content)
            } else if dimension.is_max_content() {
                Some(max_content)
            } else if dimension.is_fit_content() {
                Some(fit_content())
            } else if dimension.is_stretch() {
                match available {
                    AvailableSpace::Definite(value) => Some(value.max(0.0)),
                    AvailableSpace::MinContent => Some(min_content),
                    AvailableSpace::MaxContent => Some(max_content),
                }
            } else {
                dimension
                    .maybe_resolve(percentage_basis, resolve_stylo_calc_value)
                    .map(|size| size + box_sizing_adjustment)
            }
        };

        let authored_sizes_apply = inputs.sizing_mode == SizingMode::InherentSize;
        let preferred = authored_sizes_apply
            .then(|| resolve_dimension(logical_size.inline_size))
            .flatten();
        let min_size = authored_sizes_apply
            .then(|| resolve_dimension(logical_min_size.inline_size))
            .flatten();
        let max_size = authored_sizes_apply
            .then(|| resolve_dimension(logical_max_size.inline_size))
            .flatten();

        let mut used = space
            .known_size
            .inline_size
            .or(preferred)
            .unwrap_or_else(fit_content);
        if !self.layout_mode.is_fixed() {
            used = used.max(min_content);
        }
        if let Some(max_size) = max_size {
            used = used.min(max_size);
        }
        if let Some(min_size) = min_size {
            used = used.max(min_size);
        }
        used.max(inline_insets)
    }

    fn fixed_grid_min_border_box_size(&self) -> Option<f32> {
        if !self.layout_mode.is_fixed() {
            return None;
        }

        let padding = self
            .style
            .padding
            .resolve_or_zero(None, resolve_stylo_calc_value);
        let border = self
            .style
            .border
            .resolve_or_zero(None, resolve_stylo_calc_value);
        let inline_insets = padding.left + padding.right + border.left + border.right;
        let internal_spacing =
            self.inline_border_spacing.max(0.0) * self.column_count.saturating_sub(1) as f32;
        Some(
            fixed_grid_min_inline_size(&self.column_constraints) + inline_insets + internal_spacing,
        )
    }

    /// Tables cannot shrink their grid box below the rows' minimum extent.
    ///
    /// Grid has already measured each cell and exposes the resulting row
    /// tracks here, even when a smaller known table block size constrained the
    /// container. Blink applies the same floor in `EndTableBoxLayout` by
    /// taking the maximum of the laid-out sections and the CSS table size.
    fn encompass_minimum_row_block_size(&self, output: &mut LayoutOutput) {
        let Some(detailed) = self.detailed.as_ref() else {
            return;
        };
        if detailed.rows.sizes.is_empty() {
            return;
        }

        let logical_output_size = self.writing_mode.to_logical(output.size);
        let percentage_basis = Some(logical_output_size.inline_size);
        let padding = self
            .style
            .padding
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let border = self
            .style
            .border
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let minimum_block_size = track_extent(&detailed.rows.sizes, &detailed.rows.gutters)
            + physical_block_sum(self.writing_mode, padding)
            + physical_block_sum(self.writing_mode, border);
        if logical_output_size.block_size >= minimum_block_size {
            return;
        }

        output.size = self.writing_mode.to_physical(LogicalSize {
            inline_size: logical_output_size.inline_size,
            block_size: minimum_block_size,
        });
    }

    /// Convert Grid's intrinsic row measurements into CSS table row sizes.
    ///
    /// Grid owns cell measurement and placement, but its `align-content:
    /// stretch` rule distributes free block space equally across auto tracks.
    /// CSS tables instead classify rows and row groups as percentage, fixed,
    /// or automatic, then distribute excess block size by that hierarchy.
    /// The first Grid pass runs with start alignment to expose row minima; a
    /// second pass consumes the final fixed tracks produced here.
    fn resolve_row_tracks(&mut self, grid_size: Size<f32>) -> bool {
        let Some(detailed) = self.detailed.as_ref() else {
            return false;
        };
        if detailed.rows.sizes.len() != self.rows.len() || self.rows.is_empty() {
            return false;
        }

        let logical_grid_size = self.writing_mode.to_logical(grid_size);
        let percentage_basis = Some(logical_grid_size.inline_size);
        let padding = self
            .style
            .padding
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let border = self
            .style
            .border
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let grid_block_size = (logical_grid_size.block_size
            - physical_block_sum(self.writing_mode, padding)
            - physical_block_sum(self.writing_mode, border))
        .max(0.0);
        let mut row_sizes = detailed.rows.sizes.clone();
        let original_row_sizes = row_sizes.clone();

        for section in &self.sections {
            let Some(fixed_block_size) = section.fixed_block_size else {
                continue;
            };
            let current = row_range_block_size(
                &row_sizes,
                section.start_row,
                section.row_count,
                self.block_border_spacing,
            );
            if fixed_block_size > current {
                distribute_excess_block_size_to_rows(
                    section.start_row,
                    section.row_count,
                    fixed_block_size,
                    false,
                    self.block_border_spacing,
                    Some(fixed_block_size),
                    &self.rows,
                    &mut row_sizes,
                );
            }
        }

        distribute_table_block_size_to_sections(
            self.block_border_spacing,
            grid_block_size,
            &self.sections,
            &self.rows,
            &mut row_sizes,
        );

        let changed = original_row_sizes
            .iter()
            .zip(&row_sizes)
            .any(|(before, after)| (before - after).abs() > f32::EPSILON);
        if changed {
            self.style.grid_template_rows = row_sizes
                .into_iter()
                .map(|size| {
                    let track: taffy::TrackSizingFunction = style_helpers::length(size.max(0.0));
                    track.into()
                })
                .collect();
        }
        changed
    }
}

fn row_range_block_size(
    row_sizes: &[f32],
    start_row: usize,
    row_count: usize,
    border_spacing: f32,
) -> f32 {
    let end_row = start_row.saturating_add(row_count).min(row_sizes.len());
    if start_row >= end_row {
        return 0.0;
    }
    row_sizes[start_row..end_row].iter().sum::<f32>()
        + border_spacing.max(0.0) * (end_row - start_row - 1) as f32
}

#[allow(clippy::too_many_arguments)]
fn distribute_excess_block_size_to_rows(
    start_row: usize,
    row_count: usize,
    desired_block_size: f32,
    is_rowspan_distribution: bool,
    border_spacing: f32,
    percentage_resolution_block_size: Option<f32>,
    rows: &[TableRow],
    row_sizes: &mut [f32],
) {
    if row_count == 0 || start_row >= rows.len() || start_row >= row_sizes.len() {
        return;
    }
    let end_row = start_row
        .saturating_add(row_count)
        .min(rows.len())
        .min(row_sizes.len());
    let row_count = end_row - start_row;
    let target_row_sum = (desired_block_size.max(0.0)
        - border_spacing.max(0.0) * row_count.saturating_sub(1) as f32)
        .max(0.0);
    let mut total_block_size = row_sizes[start_row..end_row].iter().sum::<f32>();
    let mut distributable = target_row_sum - total_block_size;
    if distributable <= 0.0 {
        return;
    }

    let mut rows_with_originating_rowspan = Vec::new();
    let mut percent_rows_with_deficit = Vec::new();
    let mut percent_deficits = Vec::new();
    let mut unconstrained_non_empty_rows = Vec::new();
    let mut empty_rows = Vec::new();
    let mut unconstrained_empty_rows = Vec::new();
    let mut non_empty_rows = Vec::new();
    let mut constrained_non_empty_row_count = 0usize;

    for index in start_row..end_row {
        let row = &rows[index];
        if is_rowspan_distribution && index != start_row && row.has_rowspan_start {
            rows_with_originating_rowspan.push(index);
        }

        let mut is_empty = row_sizes[index] == 0.0;
        if let (Some(percent), Some(percentage_basis)) =
            (row.percent, percentage_resolution_block_size)
        {
            let deficit = (percent * percentage_basis - row_sizes[index]).max(0.0);
            if percent != 0.0 && deficit > 0.0 {
                percent_rows_with_deficit.push(index);
                percent_deficits.push(deficit);
                is_empty = false;
            }
        }

        let is_constrained = row.is_constrained
            && (row.percent.is_none() || percentage_resolution_block_size.is_some());
        if is_empty {
            empty_rows.push(index);
            if !is_constrained {
                unconstrained_empty_rows.push(index);
            }
        } else {
            non_empty_rows.push(index);
            if is_constrained {
                constrained_non_empty_row_count += 1;
            } else {
                unconstrained_non_empty_rows.push(index);
            }
        }
    }

    if !percent_rows_with_deficit.is_empty() {
        let total_deficit = percent_deficits.iter().sum::<f32>();
        let percent_distributable = distributable.min(total_deficit);
        let distributed = grow_weighted(
            row_sizes,
            &percent_rows_with_deficit,
            &percent_deficits,
            percent_distributable,
        );
        distributable -= distributed;
        total_block_size += distributed;
        if distributable <= 0.0 {
            return;
        }
    }

    if !rows_with_originating_rowspan.is_empty() {
        grow_evenly(row_sizes, &rows_with_originating_rowspan, distributable);
        return;
    }

    if !unconstrained_non_empty_rows.is_empty() {
        let weights = unconstrained_non_empty_rows
            .iter()
            .map(|index| row_sizes[*index])
            .collect::<Vec<_>>();
        grow_weighted(
            row_sizes,
            &unconstrained_non_empty_rows,
            &weights,
            distributable,
        );
        return;
    }

    if !empty_rows.is_empty() {
        let has_only_empty_rows = empty_rows.len() == row_count;
        if is_rowspan_distribution && has_only_empty_rows {
            row_sizes[*empty_rows.last().expect("non-empty row index list")] += distributable;
            return;
        }
        if !is_rowspan_distribution
            && (has_only_empty_rows
                || empty_rows.len() + constrained_non_empty_row_count == row_count)
        {
            let rows_to_grow = if unconstrained_empty_rows.is_empty() {
                &empty_rows
            } else {
                &unconstrained_empty_rows
            };
            grow_evenly(row_sizes, rows_to_grow, distributable);
            return;
        }
    }

    if !non_empty_rows.is_empty() {
        let weights = non_empty_rows
            .iter()
            .map(|index| row_sizes[*index])
            .collect::<Vec<_>>();
        let weight_sum = weights.iter().sum::<f32>();
        if weight_sum > 0.0 {
            grow_weighted(row_sizes, &non_empty_rows, &weights, distributable);
        } else if total_block_size == 0.0 {
            grow_evenly(row_sizes, &non_empty_rows, distributable);
        }
    }
}

fn distribute_table_block_size_to_sections(
    border_spacing: f32,
    table_grid_block_size: f32,
    sections: &[TableSection],
    rows: &[TableRow],
    row_sizes: &mut [f32],
) {
    if sections.is_empty() {
        return;
    }

    let border_spacing = border_spacing.max(0.0);
    let target_section_sum = (table_grid_block_size.max(0.0)
        - border_spacing * sections.len().saturating_sub(1) as f32)
        .max(0.0);
    let mut section_sizes = sections
        .iter()
        .map(|section| {
            row_range_block_size(
                row_sizes,
                section.start_row,
                section.row_count,
                border_spacing,
            )
        })
        .collect::<Vec<_>>();
    let minimum_size = section_sizes.iter().sum::<f32>();
    if target_section_sum <= minimum_size {
        return;
    }

    let mut remaining = target_section_sum - minimum_size;
    let percent_indices = sections
        .iter()
        .enumerate()
        .filter_map(|(index, section)| section.percent.map(|_| index))
        .collect::<Vec<_>>();
    if !percent_indices.is_empty() {
        let percent_deficits = percent_indices
            .iter()
            .map(|index| {
                let section = &sections[*index];
                (section.percent.unwrap_or(0.0) * target_section_sum - section_sizes[*index])
                    .max(0.0)
            })
            .collect::<Vec<_>>();
        let total_deficit = percent_deficits.iter().sum::<f32>();
        if total_deficit > 0.0 {
            let distributed = grow_weighted(
                &mut section_sizes,
                &percent_indices,
                &percent_deficits,
                remaining.min(total_deficit),
            );
            remaining -= distributed;
        }
    }

    if remaining > 0.0 {
        let has_tbody = sections.iter().any(|section| section.is_tbody);
        let eligible = |section: &TableSection| !has_tbody || section.is_tbody;
        let automatic = sections
            .iter()
            .enumerate()
            .filter_map(|(index, section)| {
                (eligible(section) && section.percent.is_none() && !section.is_constrained)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        let fixed = sections
            .iter()
            .enumerate()
            .filter_map(|(index, section)| {
                (eligible(section) && section.percent.is_none() && section.is_constrained)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        let percentage = sections
            .iter()
            .enumerate()
            .filter_map(|(index, section)| {
                (eligible(section) && section.percent.is_some()).then_some(index)
            })
            .collect::<Vec<_>>();
        let recipients = if !automatic.is_empty() {
            automatic
        } else if !fixed.is_empty() {
            fixed
        } else {
            percentage
        };
        if !recipients.is_empty() {
            let weights = recipients
                .iter()
                .map(|index| section_sizes[*index])
                .collect::<Vec<_>>();
            if weights.iter().sum::<f32>() > 0.0 {
                grow_weighted(&mut section_sizes, &recipients, &weights, remaining);
            } else {
                grow_evenly(&mut section_sizes, &recipients, remaining);
            }
        }
    }

    for (section, desired_block_size) in sections.iter().zip(section_sizes) {
        let current = row_range_block_size(
            row_sizes,
            section.start_row,
            section.row_count,
            border_spacing,
        );
        if desired_block_size > current {
            distribute_excess_block_size_to_rows(
                section.start_row,
                section.row_count,
                desired_block_size,
                false,
                border_spacing,
                Some(desired_block_size),
                rows,
                row_sizes,
            );
        }
    }
}

fn grow_weighted(sizes: &mut [f32], indices: &[usize], weights: &[f32], amount: f32) -> f32 {
    if amount <= 0.0 || indices.is_empty() || indices.len() != weights.len() {
        return 0.0;
    }
    let total_weight = weights.iter().sum::<f32>();
    if total_weight <= 0.0 {
        return grow_evenly(sizes, indices, amount);
    }

    let mut remaining = amount;
    for (position, (index, weight)) in indices.iter().zip(weights).enumerate() {
        let delta = if position + 1 == indices.len() {
            remaining
        } else {
            amount * weight.max(0.0) / total_weight
        };
        if let Some(size) = sizes.get_mut(*index) {
            *size += delta;
            remaining -= delta;
        }
    }
    amount - remaining
}

fn grow_evenly(sizes: &mut [f32], indices: &[usize], amount: f32) -> f32 {
    if amount <= 0.0 || indices.is_empty() {
        return 0.0;
    }
    let share = amount / indices.len() as f32;
    let mut remaining = amount;
    for (position, index) in indices.iter().enumerate() {
        let delta = if position + 1 == indices.len() {
            remaining
        } else {
            share
        };
        if let Some(size) = sizes.get_mut(*index) {
            *size += delta;
            remaining -= delta;
        }
    }
    amount - remaining
}

fn collect_columns<N>(
    world: &LayoutWorld<N>,
    current: LayoutBoxId,
    group: Option<LayoutBoxId>,
    columns: &mut Vec<TableColumn>,
    tracks: &mut Vec<TableColumnConstraint>,
) where
    N: Copy + Debug + Eq + Hash,
{
    match world.boxes[current.index()].kind {
        LayoutBoxKind::TableColumnGroup => {
            let before = tracks.len();
            for child in world.boxes[current.index()].children.iter().copied() {
                collect_columns(world, child, Some(current), columns, tracks);
            }
            if tracks.len() == before {
                let span = table_data(world, current).span.max(1) as usize;
                let track = dimension_track(world.boxes[current.index()].style.taffy.size.width);
                tracks.extend(std::iter::repeat_n(track, span));
                columns.push(TableColumn {
                    id: current,
                    group: None,
                    start: before,
                    span,
                });
            }
        }
        LayoutBoxKind::TableColumn => {
            let span = table_data(world, current).span.max(1) as usize;
            let start = tracks.len();
            let track = dimension_track(world.boxes[current.index()].style.taffy.size.width);
            tracks.extend(std::iter::repeat_n(track, span));
            columns.push(TableColumn {
                id: current,
                group,
                start,
                span,
            });
        }
        _ => {}
    }
}

fn collect_rows<N>(
    world: &LayoutWorld<N>,
    current: LayoutBoxId,
    group: Option<LayoutBoxId>,
    table_writing_mode: WritingMode,
    rows: &mut Vec<TableRow>,
    cells: &mut Vec<TableCell>,
) where
    N: Copy + Debug + Eq + Hash,
{
    match world.boxes[current.index()].kind {
        LayoutBoxKind::TableRowGroup
        | LayoutBoxKind::TableHeaderGroup
        | LayoutBoxKind::TableFooterGroup
        | LayoutBoxKind::AnonymousTableRowGroup => {
            for child in world.boxes[current.index()].children.iter().copied() {
                collect_rows(world, child, Some(current), table_writing_mode, rows, cells);
            }
        }
        LayoutBoxKind::TableRow | LayoutBoxKind::AnonymousTableRow => {
            let row_index = rows.len();
            let row_dimension = table_writing_mode
                .to_logical(world.boxes[current.index()].style.taffy.size)
                .block_size;
            let (_, mut row_percent, mut is_constrained) = table_block_constraint(row_dimension);
            let mut has_rowspan_start = false;
            for cell in world.boxes[current.index()].children.iter().copied() {
                if !matches!(
                    world.boxes[cell.index()].kind,
                    LayoutBoxKind::TableCell | LayoutBoxKind::AnonymousTableCell
                ) {
                    continue;
                }
                let data = table_data(world, cell);
                let column_span = usize::from(data.column_span.max(1));
                let row_span = usize::from(data.row_span.max(1));
                let authored_style = &world.boxes[cell.index()].style;
                let mut cell_style = authored_style.taffy.clone();
                let cell_block_size = table_writing_mode.to_logical(cell_style.size).block_size;
                let (_, cell_percent, cell_is_constrained) =
                    table_block_constraint(cell_block_size);
                if row_span == 1 {
                    is_constrained |= cell_is_constrained;
                    row_percent = max_optional(row_percent, cell_percent);
                } else {
                    has_rowspan_start = true;
                }
                cell_style.margin = Rect::ZERO.map(style_helpers::length);
                if cell_style.align_content.is_none() {
                    cell_style.align_content = match authored_style.vertical_align().kind {
                        LayoutInlineAlignment::Middle => Some(taffy::AlignContent::CENTER),
                        LayoutInlineAlignment::Bottom => Some(taffy::AlignContent::END),
                        LayoutInlineAlignment::Top => Some(taffy::AlignContent::START),
                        LayoutInlineAlignment::Baseline
                        | LayoutInlineAlignment::TextTop
                        | LayoutInlineAlignment::TextBottom => None,
                    };
                }
                cells.push(TableCell {
                    id: cell,
                    style: cell_style,
                    row: row_index,
                    column: 0,
                    row_span,
                    column_span,
                });
            }
            rows.push(TableRow {
                id: current,
                group,
                index: row_index,
                track: minimum_dimension_track(row_dimension),
                percent: row_percent,
                is_constrained,
                has_rowspan_start,
            });
        }
        _ => {}
    }
}

fn place_table_cells(cells: &mut [TableCell], rows: &[TableRow], max_columns: &mut usize) {
    let mut occupied_until = Vec::<usize>::new();
    let mut active_group = None;

    for row in rows {
        if active_group != Some(row.group) {
            occupied_until.clear();
            active_group = Some(row.group);
        }
        let section_end = rows
            .iter()
            .skip(row.index + 1)
            .find(|candidate| candidate.group != row.group)
            .map_or(rows.len(), |candidate| candidate.index);
        let mut cursor = 0usize;
        for cell in cells.iter_mut().filter(|cell| cell.row == row.index) {
            let span = cell.column_span.max(1);
            loop {
                let end = cursor.saturating_add(span);
                if occupied_until.len() < end {
                    occupied_until.resize(end, 0);
                }
                if occupied_until[cursor..end]
                    .iter()
                    .all(|occupied| *occupied <= row.index)
                {
                    cell.column = cursor;
                    cell.row_span = cell
                        .row_span
                        .min(section_end.saturating_sub(row.index))
                        .max(1);
                    for occupied in &mut occupied_until[cursor..end] {
                        *occupied = row.index.saturating_add(cell.row_span);
                    }
                    cursor = end;
                    *max_columns = (*max_columns).max(end);
                    break;
                }
                cursor += 1;
            }
        }
    }
}

fn collect_cell_constraint(
    cell: &TableCell,
    constraint: TableCellInlineConstraint,
    cell_constraints: &mut [Option<TableCellInlineConstraint>],
    cell_spans: &mut Vec<TableCellSpanConstraint>,
) {
    if cell.column_span == 1 {
        let Some(slot) = cell_constraints.get_mut(cell.column) else {
            return;
        };
        if let Some(existing) = slot {
            existing.encompass(constraint);
        } else {
            *slot = Some(constraint);
        }
    } else {
        cell_spans.push(TableCellSpanConstraint {
            start_column: cell.column,
            span: cell.column_span,
            cell: constraint,
        });
    }
}

fn table_data<N>(world: &LayoutWorld<N>, id: LayoutBoxId) -> crate::LayoutTableData
where
    N: Copy + Debug + Eq + Hash,
{
    world.boxes[id.index()]
        .element_semantics
        .as_ref()
        .and_then(|semantics| semantics.metadata.table)
        .unwrap_or_default()
}

fn dimension_track(dimension: Dimension) -> TableColumnConstraint {
    match dimension.tag() {
        taffy::CompactLength::LENGTH_TAG => TableColumnConstraint::length(dimension.value()),
        taffy::CompactLength::PERCENT_TAG => TableColumnConstraint::percent(dimension.value(), 0.0),
        _ => TableColumnConstraint::explicit_auto(),
    }
}

fn minimum_dimension_track(dimension: Dimension) -> taffy::TrackSizingFunction {
    match dimension.tag() {
        taffy::CompactLength::LENGTH_TAG => style_helpers::minmax(
            style_helpers::length(dimension.value()),
            style_helpers::auto(),
        ),
        taffy::CompactLength::PERCENT_TAG => style_helpers::minmax(
            style_helpers::percent(dimension.value()),
            style_helpers::auto(),
        ),
        _ => style_helpers::auto(),
    }
}

fn table_block_constraint(dimension: Dimension) -> (Option<f32>, Option<f32>, bool) {
    match dimension.tag() {
        taffy::CompactLength::LENGTH_TAG => (Some(dimension.value().max(0.0)), None, true),
        taffy::CompactLength::PERCENT_TAG => (None, Some(dimension.value().max(0.0)), true),
        _ => (None, None, false),
    }
}

fn max_optional(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn clamp_section_row_percentages(rows: &mut [TableRow]) {
    let mut claimed = 0.0f32;
    for row in rows {
        let Some(percent) = row.percent else {
            continue;
        };
        let percent = percent.min((1.0 - claimed).max(0.0));
        row.percent = Some(percent);
        claimed += percent;
    }
}

fn authored_table_cell_inline_constraint(
    style: &Style<Atom>,
    table_writing_mode: WritingMode,
    mode: TableLayoutMode,
) -> TableCellInlineConstraint {
    let padding = style
        .padding
        .resolve_or_zero(None, resolve_stylo_calc_value);
    let border = style.border.resolve_or_zero(None, resolve_stylo_calc_value);
    let inline_insets = physical_inline_sum(table_writing_mode, padding)
        + physical_inline_sum(table_writing_mode, border);
    let logical_size = table_writing_mode.to_logical(style.size);
    let preferred = outer_fixed_size(logical_size.inline_size, inline_insets, style.box_sizing);
    let percent = (logical_size.inline_size.tag() == taffy::CompactLength::PERCENT_TAG)
        .then(|| logical_size.inline_size.value().max(0.0));
    let percent_border_padding =
        if mode.is_fixed() && percent.is_some() && style.box_sizing == taffy::BoxSizing::ContentBox
        {
            inline_insets
        } else {
            0.0
        };
    TableCellInlineConstraint {
        min_inline_size: 0.0,
        max_inline_size: preferred.unwrap_or(percent_border_padding),
        percent,
        percent_border_padding,
        is_constrained: preferred.is_some(),
    }
}

fn table_cell_inline_constraint<N>(
    world: &mut LayoutWorld<N>,
    cell: LayoutBoxId,
    table_writing_mode: WritingMode,
    mode: TableLayoutMode,
) -> TableCellInlineConstraint
where
    N: Copy + Debug + Eq + Hash,
{
    let style = world.boxes[cell.index()].style.taffy.clone();
    let padding = style
        .padding
        .resolve_or_zero(None, resolve_stylo_calc_value);
    let border = style.border.resolve_or_zero(None, resolve_stylo_calc_value);
    let inline_insets = physical_inline_sum(table_writing_mode, padding)
        + physical_inline_sum(table_writing_mode, border);
    let logical_size = table_writing_mode.to_logical(style.size);
    let logical_min_size = table_writing_mode.to_logical(style.min_size);
    let logical_max_size = table_writing_mode.to_logical(style.max_size);
    let preferred = outer_fixed_size(logical_size.inline_size, inline_insets, style.box_sizing);
    let css_min = outer_fixed_size(
        logical_min_size.inline_size,
        inline_insets,
        style.box_sizing,
    );
    let css_max = outer_fixed_size(
        logical_max_size.inline_size,
        inline_insets,
        style.box_sizing,
    );
    let percent = (logical_size.inline_size.tag() == taffy::CompactLength::PERCENT_TAG)
        .then(|| logical_size.inline_size.value().max(0.0));

    let (content_min, content_max) = if mode.is_fixed() {
        let max = if preferred.is_none() {
            measure_table_cell_intrinsic_inline_size(
                world,
                cell,
                table_writing_mode,
                AvailableSpace::MaxContent,
            )
        } else {
            0.0
        };
        (0.0, max)
    } else {
        (
            measure_table_cell_intrinsic_inline_size(
                world,
                cell,
                table_writing_mode,
                AvailableSpace::MinContent,
            ),
            measure_table_cell_intrinsic_inline_size(
                world,
                cell,
                table_writing_mode,
                AvailableSpace::MaxContent,
            ),
        )
    };

    let mut min_inline_size = if mode.is_fixed() {
        0.0
    } else {
        content_min.max(css_min.unwrap_or(0.0))
    };
    let mut content_max = preferred.unwrap_or(content_max);
    if let Some(css_max) = css_max {
        content_max = content_max.min(css_max);
        min_inline_size = min_inline_size.min(css_max);
    }
    let max_inline_size = min_inline_size.max(content_max);
    let percent_border_padding =
        if mode.is_fixed() && percent.is_some() && style.box_sizing == taffy::BoxSizing::ContentBox
        {
            inline_insets
        } else {
            0.0
        };

    TableCellInlineConstraint {
        min_inline_size,
        max_inline_size,
        percent,
        percent_border_padding,
        is_constrained: preferred.is_some(),
    }
}

fn measure_table_cell_intrinsic_inline_size<N>(
    world: &mut LayoutWorld<N>,
    cell: LayoutBoxId,
    table_writing_mode: WritingMode,
    available_inline_size: AvailableSpace,
) -> f32
where
    N: Copy + Debug + Eq + Hash,
{
    let available_space = table_writing_mode.to_physical(LogicalSize {
        inline_size: available_inline_size,
        block_size: AvailableSpace::MaxContent,
    });
    let intrinsic_inputs = LayoutInput {
        known_dimensions: Size::NONE,
        definite_dimensions: Size::NONE,
        parent_size: Size::NONE,
        parent_writing_mode: table_writing_mode,
        available_space,
        ignored_margins_for_stretch: Rect::default(),
        sizing_mode: SizingMode::ContentSize,
        sizing_purpose: SizingPurpose::IntrinsicContribution,
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::from(table_writing_mode.inline_axis()),
        inline_auto_behavior: AutoSizeBehavior::FitContent,
        block_auto_behavior: AutoSizeBehavior::FitContent,
        orthogonal_fallback: OrthogonalFallback::UseInitialContainingBlock,
        vertical_margins_are_collapsible: Line::FALSE,
    };
    table_writing_mode
        .to_logical(
            world
                .compute_child_size(cell.to_taffy(), intrinsic_inputs)
                .size,
        )
        .inline_size
        .max(0.0)
}

fn outer_fixed_size(
    dimension: Dimension,
    inline_insets: f32,
    box_sizing: taffy::BoxSizing,
) -> Option<f32> {
    (dimension.tag() == taffy::CompactLength::LENGTH_TAG).then(|| {
        if box_sizing == taffy::BoxSizing::ContentBox {
            dimension.value().max(0.0) + inline_insets
        } else {
            dimension.value().max(0.0).max(inline_insets)
        }
    })
}

fn physical_inline_sum(writing_mode: WritingMode, rect: Rect<f32>) -> f32 {
    if writing_mode.is_horizontal() {
        rect.left + rect.right
    } else {
        rect.top + rect.bottom
    }
}

fn physical_block_sum(writing_mode: WritingMode, rect: Rect<f32>) -> f32 {
    if writing_mode.is_horizontal() {
        rect.top + rect.bottom
    } else {
        rect.left + rect.right
    }
}

fn clear_table_cell_inline_sizing(style: &mut Style<Atom>, writing_mode: WritingMode) {
    set_physical_inline_dimension(writing_mode, &mut style.size, Dimension::auto());
    set_physical_inline_dimension(writing_mode, &mut style.min_size, Dimension::auto());
    set_physical_inline_dimension(writing_mode, &mut style.max_size, Dimension::auto());
}

fn normalize_table_cell_block_sizing(style: &mut Style<Atom>, writing_mode: WritingMode) {
    let size = writing_mode.to_logical(style.size).block_size;
    let min_size = writing_mode.to_logical(style.min_size).block_size;
    if writing_mode.is_horizontal() {
        if min_size.is_auto() {
            style.min_size.height = size;
        }
        style.size.height = Dimension::auto();
    } else {
        if min_size.is_auto() {
            style.min_size.width = size;
        }
        style.size.width = Dimension::auto();
    }
}

fn set_physical_inline_dimension(
    writing_mode: WritingMode,
    size: &mut Size<Dimension>,
    value: Dimension,
) {
    if writing_mode.is_horizontal() {
        size.width = value;
    } else {
        size.height = value;
    }
}

fn layout_captions<N>(
    world: &mut LayoutWorld<N>,
    captions: &[LayoutBoxId],
    width: f32,
    mut y: f32,
    parent_writing_mode: WritingMode,
) -> f32
where
    N: Copy + Debug + Eq + Hash,
{
    let start = y;
    for (order, caption) in captions.iter().copied().enumerate() {
        let style = world.boxes[caption.index()].style.taffy.clone();
        let margin = style
            .margin
            .resolve_or_zero(Some(width), resolve_stylo_calc_value);
        y += margin.top;
        let inputs = LayoutInput {
            known_dimensions: Size {
                width: Some((width - margin.left - margin.right).max(0.0)),
                height: None,
            },
            definite_dimensions: Size {
                width: Some((width - margin.left - margin.right).max(0.0)),
                height: None,
            },
            parent_size: Size {
                width: Some(width),
                height: None,
            },
            parent_writing_mode,
            available_space: Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::MaxContent,
            },
            ignored_margins_for_stretch: Rect::default(),
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode: RunMode::PerformLayout,
            axis: taffy::RequestedAxis::Both,
            inline_auto_behavior: AutoSizeBehavior::FitContent,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            orthogonal_fallback: OrthogonalFallback::UseInitialContainingBlock,
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let output = world.compute_child_layout(caption.to_taffy(), inputs);
        set_box_layout(
            world,
            caption,
            Point { x: margin.left, y },
            output,
            order,
            Some(width),
        );
        y += output.size.height + margin.bottom;
    }
    y - start
}

fn shift_grid_children<N>(world: &mut LayoutWorld<N>, cells: &[TableCell], offset: f32)
where
    N: Copy + Debug + Eq + Hash,
{
    if offset == 0.0 {
        return;
    }
    for cell in cells {
        world.boxes[cell.id.index()].unrounded_layout.location.y += offset;
    }
}

fn apply_structural_layout<N>(
    world: &mut LayoutWorld<N>,
    root: LayoutBoxId,
    context: &TableContext,
    top_offset: f32,
    grid_size: Size<f32>,
) where
    N: Copy + Debug + Eq + Hash,
{
    let root_style = &context.style;
    let padding = root_style
        .padding
        .resolve_or_zero(Some(grid_size.width), resolve_stylo_calc_value);
    let border = root_style
        .border
        .resolve_or_zero(Some(grid_size.width), resolve_stylo_calc_value);
    let origin = Point {
        x: border.left + padding.left,
        y: top_offset + border.top + padding.top,
    };
    let Some(detailed) = context.detailed.as_ref() else {
        return;
    };
    let row_starts = track_starts(origin.y, &detailed.rows.sizes, &detailed.rows.gutters);
    let column_starts = track_starts(origin.x, &detailed.columns.sizes, &detailed.columns.gutters);
    let content_width = track_extent(&detailed.columns.sizes, &detailed.columns.gutters);
    let content_height = track_extent(&detailed.rows.sizes, &detailed.rows.gutters);
    if context.collapsed_borders {
        let mut row_lines = row_starts.clone();
        row_lines.push(origin.y + content_height);
        let mut column_lines = column_starts.clone();
        column_lines.push(origin.x + content_width);
        set_collapsed_border_geometry(world, root, &column_lines, &row_lines);
    }

    for row in &context.rows {
        let y = row_starts.get(row.index).copied().unwrap_or(origin.y);
        let height = detailed.rows.sizes.get(row.index).copied().unwrap_or(0.0);
        set_table_part_layout(world, row.id, origin.x, y, content_width, height);
    }
    let mut groups = context
        .rows
        .iter()
        .filter_map(|row| row.group)
        .collect::<Vec<_>>();
    groups.sort_by_key(|id| id.index());
    groups.dedup();
    for group in groups {
        let group_rows = context.rows.iter().filter(|row| row.group == Some(group));
        let mut start = usize::MAX;
        let mut end = 0usize;
        for row in group_rows {
            start = start.min(row.index);
            end = end.max(row.index + 1);
        }
        if start != usize::MAX {
            let y = row_starts.get(start).copied().unwrap_or(origin.y);
            let height =
                track_range_extent(&detailed.rows.sizes, &detailed.rows.gutters, start, end);
            set_table_part_layout(world, group, origin.x, y, content_width, height);
        }
    }
    for column in &context.columns {
        let x = column_starts.get(column.start).copied().unwrap_or(origin.x);
        let width = track_range_extent(
            &detailed.columns.sizes,
            &detailed.columns.gutters,
            column.start,
            column.start.saturating_add(column.span),
        );
        set_table_part_layout(world, column.id, x, origin.y, width, content_height);
    }
    let mut column_groups = context
        .columns
        .iter()
        .filter_map(|column| column.group)
        .collect::<Vec<_>>();
    column_groups.sort_by_key(|id| id.index());
    column_groups.dedup();
    for group in column_groups {
        let grouped = context
            .columns
            .iter()
            .filter(|column| column.group == Some(group));
        let mut start = usize::MAX;
        let mut end = 0usize;
        for column in grouped {
            start = start.min(column.start);
            end = end.max(column.start.saturating_add(column.span));
        }
        if start != usize::MAX {
            let x = column_starts.get(start).copied().unwrap_or(origin.x);
            let width = track_range_extent(
                &detailed.columns.sizes,
                &detailed.columns.gutters,
                start,
                end,
            );
            set_table_part_layout(world, group, x, origin.y, width, content_height);
        }
    }

    // Keep the root in the numeric tree even for an empty table.
    let _ = root;
}

fn track_starts(origin: f32, sizes: &[f32], gutters: &[f32]) -> Vec<f32> {
    let mut starts = Vec::with_capacity(sizes.len());
    let mut cursor = origin + gutters.first().copied().unwrap_or(0.0);
    for (index, size) in sizes.iter().copied().enumerate() {
        starts.push(cursor);
        cursor += size + gutters.get(index + 1).copied().unwrap_or(0.0);
    }
    starts
}

fn track_extent(sizes: &[f32], gutters: &[f32]) -> f32 {
    sizes.iter().sum::<f32>() + gutters.iter().sum::<f32>()
}

fn track_range_extent(sizes: &[f32], gutters: &[f32], start: usize, end: usize) -> f32 {
    let end = end.min(sizes.len());
    if start >= end {
        return 0.0;
    }
    sizes[start..end].iter().sum::<f32>()
        + gutters
            .get(start + 1..end)
            .unwrap_or_default()
            .iter()
            .sum::<f32>()
}

fn set_table_part_layout<N>(
    world: &mut LayoutWorld<N>,
    id: LayoutBoxId,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) where
    N: Copy + Debug + Eq + Hash,
{
    // Internal table parts expose the grid's structural geometry, not an
    // ordinary CSS box model. Their margin and padding are ignored. Borders
    // are ignored in the separated model and represented by the table-owned
    // conflict grid in the collapsed model; keep the authored style intact
    // while publishing zero used decoration edges to generic paint.
    world.boxes[id.index()].unrounded_layout = Layout {
        order: 0,
        location: Point { x, y },
        size: Size { width, height },
        content_size: Size { width, height },
        scrollbar_size: Size::ZERO,
        border: Rect::ZERO,
        padding: Rect::ZERO,
        margin: Rect::ZERO,
    };
}

fn set_box_layout<N>(
    world: &mut LayoutWorld<N>,
    id: LayoutBoxId,
    location: Point<f32>,
    output: LayoutOutput,
    order: usize,
    parent_width: Option<f32>,
) where
    N: Copy + Debug + Eq + Hash,
{
    let style = &world.boxes[id.index()].style.taffy;
    let padding = style
        .padding
        .resolve_or_zero(parent_width, resolve_stylo_calc_value);
    let border = style
        .border
        .resolve_or_zero(parent_width, resolve_stylo_calc_value);
    let margin = style
        .margin
        .resolve_or_zero(parent_width, resolve_stylo_calc_value);
    world.boxes[id.index()].unrounded_layout = Layout {
        order: u32::try_from(order).unwrap_or(u32::MAX),
        location,
        size: output.size,
        content_size: output.content_size,
        scrollbar_size: Size::ZERO,
        border,
        padding,
        margin,
    };
}

fn is_table_root(kind: LayoutBoxKind) -> bool {
    matches!(
        kind,
        LayoutBoxKind::TableWrapper
            | LayoutBoxKind::InlineTableWrapper
            | LayoutBoxKind::AnonymousTableWrapper
    )
}

fn is_table_structural(kind: LayoutBoxKind) -> bool {
    matches!(
        kind,
        LayoutBoxKind::TableRowGroup
            | LayoutBoxKind::TableHeaderGroup
            | LayoutBoxKind::TableFooterGroup
            | LayoutBoxKind::TableColumnGroup
            | LayoutBoxKind::TableColumn
            | LayoutBoxKind::TableRow
            | LayoutBoxKind::AnonymousTableRowGroup
            | LayoutBoxKind::AnonymousTableRow
    )
}

struct VirtualChildIter(std::ops::Range<usize>);

impl Iterator for VirtualChildIter {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(NodeId::from)
    }
}

struct TableTreeWrapper<'a, N>
where
    N: Copy + Debug + Eq + Hash,
{
    world: &'a mut LayoutWorld<N>,
    context: &'a mut TableContext,
}

impl<N> TableTreeWrapper<'_, N>
where
    N: Copy + Debug + Eq + Hash,
{
    /// Execute one Grid child query with the pass-local table-cell style.
    /// Taffy's cache keys layout inputs rather than style identity, so clear
    /// both sides of the swap to keep authored intrinsic measurements from
    /// aliasing final Grid layout.
    fn with_grid_cell_style<R>(
        &mut self,
        cell_index: usize,
        operation: impl FnOnce(&mut LayoutWorld<N>, LayoutBoxId) -> R,
    ) -> R {
        let cell = self.context.cells[cell_index].id;
        self.world.cache_clear(cell.to_taffy());
        let authored_style = std::mem::replace(
            &mut self.world.boxes[cell.index()].style.taffy,
            self.context.cells[cell_index].style.clone(),
        );
        let result = operation(self.world, cell);
        self.world.boxes[cell.index()].style.taffy = authored_style;
        self.world.cache_clear(cell.to_taffy());
        result
    }
}

impl<N> TraversePartialTree for TableTreeWrapper<'_, N>
where
    N: Copy + Debug + Eq + Hash,
{
    type ChildIter<'a>
        = VirtualChildIter
    where
        Self: 'a;

    fn child_ids(&self, _parent_node_id: NodeId) -> Self::ChildIter<'_> {
        VirtualChildIter(0..self.context.cells.len())
    }

    fn child_count(&self, _parent_node_id: NodeId) -> usize {
        self.context.cells.len()
    }

    fn get_child_id(&self, _parent_node_id: NodeId, child_index: usize) -> NodeId {
        NodeId::from(child_index)
    }
}

impl<N> TraverseTree for TableTreeWrapper<'_, N> where N: Copy + Debug + Eq + Hash {}

impl<N> LayoutPartialTree for TableTreeWrapper<'_, N>
where
    N: Copy + Debug + Eq + Hash,
{
    type CoreContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type CustomIdent = Atom;

    fn get_core_container_style(&self, _node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        &self.context.style
    }

    fn resolve_calc_value(&self, value: *const (), basis: f32) -> f32 {
        resolve_stylo_calc_value(value, basis)
    }

    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        let cell = self.context.cells[usize::from(node_id)].id;
        self.world.boxes[cell.index()].unrounded_layout = *layout;
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let cell_index = usize::from(node_id);
        // The virtual table grid owns the used grid-item style: margins are
        // zero, column sizing has consumed every applicable inline constraint,
        // and cell block size is a minimum contribution.
        self.with_grid_cell_style(cell_index, |world, cell| {
            world.compute_child_layout(cell.to_taffy(), inputs)
        })
    }

    fn compute_child_size(&mut self, node_id: NodeId, inputs: LayoutInput) -> IntrinsicSizeResult {
        let cell_index = usize::from(node_id);
        self.with_grid_cell_style(cell_index, |world, cell| {
            world.compute_child_size(cell.to_taffy(), inputs)
        })
    }
}

impl<N> LayoutGridContainer for TableTreeWrapper<'_, N>
where
    N: Copy + Debug + Eq + Hash,
{
    type GridContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_grid_container_style(&self, _node_id: NodeId) -> Self::GridContainerStyle<'_> {
        &self.context.style
    }

    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        &self.context.cells[usize::from(child_node_id)].style
    }

    fn set_detailed_grid_info(&mut self, _node_id: NodeId, detailed_grid_info: DetailedGridInfo) {
        self.context.detailed = Some(detailed_grid_info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_block_intrinsics_remain_unresolved_until_rows_are_measured() {
        for writing_mode in [WritingMode::HorizontalTb, WritingMode::VerticalRl] {
            let mut style = Style {
                size: writing_mode.to_physical(LogicalSize {
                    inline_size: Dimension::length(101.0),
                    block_size: Dimension::max_content(),
                }),
                min_size: writing_mode.to_physical(LogicalSize {
                    inline_size: Dimension::percent(0.25),
                    block_size: Dimension::min_content(),
                }),
                max_size: writing_mode.to_physical(LogicalSize {
                    inline_size: Dimension::length(202.0),
                    block_size: Dimension::fit_content(),
                }),
                ..Style::default()
            };

            defer_table_block_intrinsics_until_row_layout(&mut style, writing_mode);

            let size = writing_mode.to_logical(style.size);
            let min_size = writing_mode.to_logical(style.min_size);
            let max_size = writing_mode.to_logical(style.max_size);
            assert_eq!(size.inline_size, Dimension::length(101.0));
            assert_eq!(min_size.inline_size, Dimension::percent(0.25));
            assert_eq!(max_size.inline_size, Dimension::length(202.0));
            assert!(size.block_size.is_auto());
            assert!(min_size.block_size.is_auto());
            assert!(max_size.block_size.is_auto());
        }
    }

    #[test]
    fn table_numeric_block_constraints_remain_parent_facing() {
        let mut style = Style {
            size: Size {
                width: Dimension::auto(),
                height: Dimension::length(150.0),
            },
            min_size: Size {
                width: Dimension::auto(),
                height: Dimension::percent(0.25),
            },
            max_size: Size {
                width: Dimension::auto(),
                height: Dimension::stretch(),
            },
            ..Style::default()
        };

        defer_table_block_intrinsics_until_row_layout(&mut style, WritingMode::HorizontalTb);

        assert_eq!(style.size.height, Dimension::length(150.0));
        assert_eq!(style.min_size.height, Dimension::percent(0.25));
        assert_eq!(style.max_size.height, Dimension::stretch());
    }

    #[test]
    fn percentage_dependent_fixed_table_has_unbounded_parent_max_content_size() {
        let grid = columns::TableGridInlineMinMax { min: 4.0, max: 4.0 };

        assert_eq!(
            TableIntrinsicInlineSizes::from_grid(
                grid,
                TableLayoutMode::Fixed,
                Dimension::percent(1.0),
            ),
            TableIntrinsicInlineSizes {
                min_content: 4.0,
                max_content: TABLE_MAX_INLINE_SIZE,
            },
        );
        assert_eq!(
            TableIntrinsicInlineSizes::from_grid(
                grid,
                TableLayoutMode::Automatic,
                Dimension::percent(1.0),
            ),
            TableIntrinsicInlineSizes {
                min_content: 4.0,
                max_content: 4.0,
            },
        );
        assert_eq!(
            TableIntrinsicInlineSizes::from_grid(
                grid,
                TableLayoutMode::Fixed,
                Dimension::length(40.0),
            ),
            TableIntrinsicInlineSizes {
                min_content: 4.0,
                max_content: 4.0,
            },
        );
    }

    fn row(percent: Option<f32>, is_constrained: bool) -> TableRow {
        TableRow {
            id: LayoutBoxId::from_index(0),
            group: None,
            index: 0,
            track: style_helpers::auto(),
            percent,
            is_constrained,
            has_rowspan_start: false,
        }
    }

    fn assert_sizes(actual_sizes: &[f32], expected_sizes: &[f32]) {
        assert_eq!(actual_sizes.len(), expected_sizes.len());
        for (actual, expected) in actual_sizes.iter().zip(expected_sizes) {
            assert!(
                (actual - expected).abs() < 0.001,
                "expected {expected}, got {actual}; sizes={actual_sizes:?}",
            );
        }
    }

    #[test]
    fn row_distribution_prefers_percentage_then_unconstrained_rows() {
        let rows = [row(Some(0.3), true), row(None, false)];
        let mut sizes = [10.0, 10.0];
        distribute_excess_block_size_to_rows(
            0,
            2,
            100.0,
            false,
            0.0,
            Some(100.0),
            &rows,
            &mut sizes,
        );
        assert_sizes(&sizes, &[30.0, 70.0]);
    }

    #[test]
    fn row_distribution_grows_only_auto_rows_before_fixed_rows() {
        let rows = [row(None, true), row(None, false)];
        let mut sizes = [30.0, 20.0];
        distribute_excess_block_size_to_rows(
            0,
            2,
            100.0,
            false,
            0.0,
            Some(100.0),
            &rows,
            &mut sizes,
        );
        assert_sizes(&sizes, &[30.0, 70.0]);
    }

    #[test]
    fn row_distribution_grows_all_fixed_rows_proportionally_as_last_resort() {
        let rows = [row(None, true), row(None, true)];
        let mut sizes = [20.0, 40.0];
        distribute_excess_block_size_to_rows(
            0,
            2,
            120.0,
            false,
            0.0,
            Some(120.0),
            &rows,
            &mut sizes,
        );
        assert_sizes(&sizes, &[40.0, 80.0]);
    }

    #[test]
    fn table_distribution_prefers_tbody_sections_and_redistributes_fixed_groups() {
        let rows = [
            row(None, true),
            row(None, false),
            row(None, false),
            row(None, false),
        ];
        let sections = [
            TableSection {
                start_row: 0,
                row_count: 1,
                fixed_block_size: None,
                percent: None,
                is_constrained: false,
                is_tbody: false,
            },
            TableSection {
                start_row: 1,
                row_count: 2,
                fixed_block_size: Some(60.0),
                percent: None,
                is_constrained: true,
                is_tbody: true,
            },
            TableSection {
                start_row: 3,
                row_count: 1,
                fixed_block_size: None,
                percent: None,
                is_constrained: false,
                is_tbody: true,
            },
        ];
        let mut sizes = [20.0, 10.0, 10.0, 10.0];
        distribute_excess_block_size_to_rows(1, 2, 60.0, false, 0.0, Some(60.0), &rows, &mut sizes);
        distribute_table_block_size_to_sections(0.0, 120.0, &sections, &rows, &mut sizes);
        assert_sizes(&sizes, &[20.0, 30.0, 30.0, 40.0]);
    }
}
