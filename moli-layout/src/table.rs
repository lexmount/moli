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
    LayoutPartialTree, Line, LogicalSize, MaybeResolve, NodeId, Point, Rect, RequestedAxis,
    ResolveOrZero, RunMode, Size, SizingMode, SizingPurpose, Style, TraversePartialTree,
    TraverseTree, WritingMode, compute_grid_layout, style_helpers,
};

use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutWorld,
    style::{LayoutInlineAlignment, resolve_stylo_calc_value},
};

mod block;
mod cache;
mod collapsed_borders;
mod columns;
mod rows;

pub(crate) use cache::TableMeasureCache;
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
    block_layout: Option<block::CellBlockLayout>,
}

#[derive(Clone, Copy)]
struct TableRow {
    id: LayoutBoxId,
    group: Option<LayoutBoxId>,
    index: usize,
    grid_index: usize,
    start_cell_index: usize,
    cell_count: usize,
    section_end: usize,
}

impl TableRow {
    fn cells(&self) -> std::ops::Range<usize> {
        self.start_cell_index..self.start_cell_index + self.cell_count
    }
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
    columns: Vec<TableColumn>,
    captions: Vec<LayoutBoxId>,
    detailed: Option<DetailedGridInfo>,
    collapsed_borders: bool,
    column_count: usize,
    column_constraints: Vec<TableColumnConstraint>,
    column_sizes: Vec<f32>,
    sections: Vec<rows::SectionConstraint>,
    section_boxes: Vec<LayoutBoxId>,
    section_tracks: Vec<std::ops::Range<usize>>,
    measured_baselines: (Option<f32>, Option<f32>),
    caption_inline_min: f32,
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
    let mut grid_inputs = context.resolve_column_tracks(inputs);
    let allocated_wrapper = world.boxes[root.index()]
        .layout_parent
        .is_some_and(|parent| {
            let display = world.boxes[parent.index()].style.display();
            display.is_flex_container() || display.is_grid_container()
        });
    if allocated_wrapper && !context.captions.is_empty() && context.writing_mode.is_horizontal() {
        let mut space = grid_inputs.constraint_space(context.writing_mode);
        if let Some(block_size) = space.known_size.block_size {
            // Flex/Grid allocate the complete wrapper. Caption space must not
            // be allocated to the table grid a second time.
            let captions = layout_captions(
                world,
                &context.captions,
                space.known_size.inline_size.unwrap_or(0.0),
                0.0,
                context.writing_mode,
                RunMode::ComputeSize,
            );
            let style = &world.boxes[root.index()].style.taffy;
            let basis = space.margin_padding_percentage_basis();
            let decorations = style
                .padding
                .resolve_or_zero(basis, resolve_stylo_calc_value)
                + style
                    .border
                    .resolve_or_zero(basis, resolve_stylo_calc_value);
            let adjustment = if style.box_sizing == taffy::BoxSizing::ContentBox {
                context
                    .writing_mode
                    .to_logical(decorations.sum_axes())
                    .block_size
            } else {
                0.0
            };
            let resolve = |dimension: Dimension| {
                dimension
                    .maybe_resolve(
                        space.percentage_resolution_size.block_size,
                        resolve_stylo_calc_value,
                    )
                    .map(|size| size + adjustment)
            };
            // The preferred table height describes the grid, excluding its
            // captions. A parent may pass that authored size as a known size
            // before it has measured the complete wrapper.
            let mut grid_minimum =
                resolve(context.writing_mode.to_logical(style.size).block_size).unwrap_or(0.0);
            if let Some(parent) = world.boxes[root.index()].layout_parent {
                let parent_style = &world.boxes[parent.index()].style;
                let main_axis = if matches!(
                    parent_style.taffy.flex_direction,
                    taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
                ) {
                    parent_style.writing_mode().inline_axis()
                } else {
                    parent_style.writing_mode().block_axis()
                };
                if parent_style.display().is_flex_container()
                    && main_axis == context.writing_mode.block_axis()
                    && style.flex_shrink == 0.0
                {
                    grid_minimum = grid_minimum.max(resolve(style.flex_basis).unwrap_or(0.0));
                }
            }
            space.known_size.block_size = Some((block_size - captions).max(grid_minimum));
            grid_inputs = space.into_layout_input();
        }
    }
    let grid_inputs = context.resolve_row_tracks(world, grid_inputs);
    let mut output = {
        let mut wrapper = TableTreeWrapper {
            world,
            context: &mut context,
        };
        compute_grid_layout(&mut wrapper, NodeId::from(0usize), grid_inputs)
    };
    if context.writing_mode.is_horizontal() && inputs.axis != RequestedAxis::Horizontal {
        // Fixed Grid tracks can return without measuring child baselines.
        // Use the row pass, including the absence of a baseline in a table
        // with no rows; final cell layout may refine it below.
        output.first_baselines.y = context.measured_baselines.0;
        output.last_baselines.y = context.measured_baselines.1;
    }

    {
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
            inputs.run_mode,
        );
        let bottom_height = layout_captions(
            world,
            &bottom_captions,
            output.size.width,
            top_height + output.size.height,
            caption_parent_writing_mode,
            inputs.run_mode,
        );
        if inputs.run_mode == RunMode::PerformLayout {
            context.align_row_baselines(world, &mut output);
            shift_grid_children(world, &context.cells, top_height);
            apply_structural_layout(world, root, &context, top_height, output.size);
        }
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
    style.grid_auto_flow = GridAutoFlow::RowDense;
    style.grid_auto_columns.clear();
    style.grid_auto_rows.clear();

    let grouped_children = TableGroupedChildren::collect(world, root);
    let mut cells = Vec::new();
    let mut rows = Vec::new();
    let mut columns = Vec::new();
    let mut max_columns = 0usize;
    let mut column_tracks = Vec::new();
    let mut sections = Vec::new();
    let mut section_boxes = Vec::new();
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
        let start = rows.len();
        collect_rows(world, section, None, &mut rows, &mut cells);
        {
            let dimension = writing_mode
                .to_logical(world.boxes[section.index()].style.taffy.size)
                .block_size;
            let is_group = matches!(
                world.boxes[section.index()].kind,
                LayoutBoxKind::TableRowGroup
                    | LayoutBoxKind::TableHeaderGroup
                    | LayoutBoxKind::TableFooterGroup
                    | LayoutBoxKind::AnonymousTableRowGroup
            );
            sections.push(rows::SectionConstraint {
                rows: start..rows.len(),
                fixed: (is_group && dimension.tag() == taffy::CompactLength::LENGTH_TAG)
                    .then(|| dimension.value().max(0.0)),
                percent: (is_group && dimension.tag() == taffy::CompactLength::PERCENT_TAG)
                    .then(|| dimension.value().max(0.0)),
                is_body: Some(section) != grouped_children.header
                    && Some(section) != grouped_children.footer,
                size: 0.0,
            });
            section_boxes.push(section);
        }
    }
    index_row_sections(&mut rows);
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
        if writing_mode.is_horizontal() {
            block::clear_cell_block_sizing(&mut cell.style, writing_mode);
        }
    }
    let placeholder_track: taffy::TrackSizingFunction = style_helpers::auto();
    style.grid_template_columns =
        std::iter::repeat_n(placeholder_track.into(), max_columns).collect();
    style.grid_template_rows = if rows.is_empty() {
        vec![style_helpers::auto()]
    } else {
        vec![style_helpers::auto(); rows.len()]
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
    let section_tracks = if writing_mode.is_horizontal() {
        Vec::new()
    } else {
        sections
            .iter()
            .map(|section| section.rows.clone())
            .collect()
    };
    TableContext {
        style,
        cells,
        rows,
        columns,
        captions: grouped_children.captions,
        detailed: None,
        collapsed_borders: collapsed,
        column_count: max_columns,
        column_constraints: column_tracks,
        column_sizes: Vec::new(),
        sections,
        section_boxes,
        section_tracks,
        measured_baselines: (None, None),
        caption_inline_min: 0.0,
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
        let first_row_cells = self
            .rows
            .first()
            .map_or(&[][..], |row| &self.cells[row.cells()]);
        for cell in first_row_cells {
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
        for &caption in &self.captions {
            let style = &world.boxes[caption.index()].style.taffy;
            let margin = style.margin.resolve_or_zero(None, resolve_stylo_calc_value);
            let output = world.compute_child_size(
                caption.to_taffy(),
                LayoutInput {
                    known_dimensions: Size::NONE,
                    definite_dimensions: Size::NONE,
                    parent_size: Size::NONE,
                    parent_writing_mode: self.writing_mode,
                    available_space: self.writing_mode.to_physical(LogicalSize {
                        inline_size: AvailableSpace::MinContent,
                        block_size: AvailableSpace::MaxContent,
                    }),
                    sizing_mode: SizingMode::InherentSize,
                    sizing_purpose: SizingPurpose::IntrinsicContribution,
                    run_mode: RunMode::ComputeSize,
                    axis: RequestedAxis::from(self.writing_mode.inline_axis()),
                    block_auto_behavior: AutoSizeBehavior::FitContent,
                    vertical_margins_are_collapsible: Line::FALSE,
                },
            );
            // Captions belong to the wrapper, but their minimum outer width
            // also constrains its grid, including anonymous tables.
            self.caption_inline_min = self.caption_inline_min.max(
                self.writing_mode.to_logical(output.size).inline_size
                    + physical_inline_sum(self.writing_mode, margin),
            );
        }
        let mut cell_constraints = vec![None; self.column_count];
        let mut cell_spans = Vec::new();
        let measured_cells = if self.layout_mode.is_fixed() {
            self.rows
                .first()
                .map_or(&[][..], |row| &self.cells[row.cells()])
        } else {
            &self.cells
        };
        for cell in measured_cells {
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
        let mut grid_min_max = compute_grid_inline_min_max(
            &self.column_constraints,
            undistributable_space,
            self.layout_mode,
        );
        grid_min_max.min = grid_min_max.min.max(self.caption_inline_min);
        grid_min_max.max = grid_min_max.max.max(self.caption_inline_min);
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
            .iter()
            .map(|&size| {
                let track: taffy::TrackSizingFunction = style_helpers::length(size);
                track.into()
            })
            .collect();
        self.column_sizes = column_sizes;

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
        used.max(inline_insets).max(self.caption_inline_min)
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
                collect_rows(world, child, Some(current), rows, cells);
            }
        }
        LayoutBoxKind::TableRow | LayoutBoxKind::AnonymousTableRow => {
            let row_index = rows.len();
            let start_cell_index = cells.len();
            for cell in world.boxes[current.index()].children.iter().copied() {
                if !matches!(
                    world.boxes[cell.index()].kind,
                    LayoutBoxKind::TableCell | LayoutBoxKind::AnonymousTableCell
                ) {
                    continue;
                }
                let data = table_data(world, cell);
                let column_span = usize::from(data.column_span.max(1));
                let row_span = usize::from(data.row_span);
                let authored_style = &world.boxes[cell.index()].style;
                let mut cell_style = authored_style.taffy.clone();
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
                    block_layout: None,
                });
            }
            rows.push(TableRow {
                id: current,
                group,
                index: row_index,
                grid_index: row_index,
                start_cell_index,
                cell_count: cells.len() - start_cell_index,
                section_end: 0,
            });
        }
        _ => {}
    }
}

/// Cache the end of each contiguous placement group in one reverse pass.
/// Consecutive ungrouped rows share a boundary, just as grouped rows do.
fn index_row_sections(rows: &mut [TableRow]) {
    let mut next_group = None;
    let mut section_end = rows.len();
    for row in rows.iter_mut().rev() {
        if next_group != Some(row.group) {
            section_end = row.index + 1;
            next_group = Some(row.group);
        }
        row.section_end = section_end;
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
        let section_end = row.section_end;
        let mut cursor = 0usize;
        for cell in &mut cells[row.cells()] {
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
                    let row_span = if cell.row_span == 0 {
                        section_end.saturating_sub(row.index)
                    } else {
                        cell.row_span
                    };
                    cell.row_span = row_span.min(section_end.saturating_sub(row.index)).max(1);
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
        sizing_mode: SizingMode::ContentSize,
        sizing_purpose: SizingPurpose::IntrinsicContribution,
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::from(table_writing_mode.inline_axis()),
        block_auto_behavior: AutoSizeBehavior::FitContent,
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

fn clear_table_cell_inline_sizing(style: &mut Style<Atom>, writing_mode: WritingMode) {
    set_physical_inline_dimension(writing_mode, &mut style.size, Dimension::auto());
    set_physical_inline_dimension(writing_mode, &mut style.min_size, Dimension::auto());
    set_physical_inline_dimension(writing_mode, &mut style.max_size, Dimension::auto());
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
    run_mode: RunMode,
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
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode,
            axis: taffy::RequestedAxis::Both,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let output = world.compute_child_layout(caption.to_taffy(), inputs);
        if run_mode == RunMode::PerformLayout {
            set_box_layout(
                world,
                caption,
                Point { x: margin.left, y },
                output,
                order,
                Some(width),
            );
        }
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
    // Empty tables retain spacing in their intrinsic inline contribution,
    // while their row/group rectangles span the content box when no real
    // columns exist. Keep that distinction out of column sizing.
    let empty_inline_spacing = if context.cells.is_empty() && context.columns.is_empty() {
        context.inline_border_spacing
    } else {
        0.0
    };
    let part_x = origin.x - empty_inline_spacing;
    let part_width = content_width + 2.0 * empty_inline_spacing;
    let occupied_sections = || {
        context
            .sections
            .iter()
            .zip(&context.section_tracks)
            .filter(|(section, _)| !section.rows.is_empty() || section.size > 0.0)
    };
    let first_section_track = occupied_sections()
        .next()
        .map_or(0, |(_, range)| range.start);
    let last_section_track = occupied_sections()
        .next_back()
        .map_or(detailed.rows.sizes.len(), |(_, range)| range.end);
    let content_top = row_starts
        .get(first_section_track)
        .copied()
        .unwrap_or(origin.y);
    let content_height = track_range_extent(
        &detailed.rows.sizes,
        &detailed.rows.gutters,
        first_section_track,
        last_section_track,
    );
    if context.collapsed_borders {
        // Border conflicts are indexed by actual rows. Empty-section tracks
        // occupy space but must not change the conflict grid's row count.
        let mut row_lines: Vec<_> = context
            .rows
            .iter()
            .map(|row| row_starts[row.grid_index])
            .collect();
        if let Some(first) = row_lines.first_mut() {
            *first = origin.y;
        }
        row_lines.push(origin.y + content_height);
        let mut column_lines = column_starts.clone();
        column_lines.push(origin.x + content_width);
        set_collapsed_border_geometry(world, root, &column_lines, &row_lines);
    }

    for row in &context.rows {
        let y = row_starts.get(row.grid_index).copied().unwrap_or(origin.y);
        let height = detailed
            .rows
            .sizes
            .get(row.grid_index)
            .copied()
            .unwrap_or(0.0);
        set_table_part_layout(world, row.id, part_x, y, part_width, height);
    }
    for (&group, tracks) in context.section_boxes.iter().zip(&context.section_tracks) {
        if matches!(
            world.boxes[group.index()].kind,
            LayoutBoxKind::TableRow | LayoutBoxKind::AnonymousTableRow
        ) {
            continue;
        }
        let y = row_starts
            .get(tracks.start)
            .copied()
            .unwrap_or(origin.y + content_height);
        let height = track_range_extent(
            &detailed.rows.sizes,
            &detailed.rows.gutters,
            tracks.start,
            tracks.end,
        );
        set_table_part_layout(world, group, part_x, y, part_width, height);
    }
    for column in &context.columns {
        let x = column_starts.get(column.start).copied().unwrap_or(origin.x);
        let width = track_range_extent(
            &detailed.columns.sizes,
            &detailed.columns.gutters,
            column.start,
            column.start.saturating_add(column.span),
        );
        set_table_part_layout(world, column.id, x, content_top, width, content_height);
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
            set_table_part_layout(world, group, x, content_top, width, content_height);
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
        let layout = self.context.cells[cell_index].block_layout;
        let mode = self.context.writing_mode;
        // The virtual table grid owns the used grid-item style: margins are
        // zero, column sizing has consumed every applicable inline constraint,
        // and cell block size is a minimum contribution.
        let output = self.with_grid_cell_style(cell_index, |world, cell| {
            block::layout_cell(world, cell, inputs, mode, layout)
        });
        if inputs.run_mode == RunMode::PerformLayout
            && let Some(layout) = &mut self.context.cells[cell_index].block_layout
            && layout.baseline.is_some()
        {
            layout.baseline = output.first_baselines.y;
        }
        output
    }

    fn compute_child_size(&mut self, node_id: NodeId, inputs: LayoutInput) -> IntrinsicSizeResult {
        self.compute_child_layout(node_id, inputs)
            .into_intrinsic_size_result()
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
    fn row_group_and_caption_sizes_agree_in_cold_measurement_and_warm_layout() {
        use crate::{LayoutDisplay, PaintColor, ResolvedLayoutStyle};

        let make_box = |kind, display, height: Option<f32>| {
            LayoutWorld::<usize>::new_box(
                None,
                None,
                None,
                "table-height-test".into(),
                None,
                None,
                None,
                kind,
                ResolvedLayoutStyle::synthetic(
                    display,
                    Style {
                        size: Size {
                            width: Dimension::length(200.0),
                            height: height.map_or(Dimension::auto(), Dimension::length),
                        },
                        ..Style::default()
                    },
                    PaintColor::TRANSPARENT,
                ),
                None,
            )
        };
        let mut world = LayoutWorld::new(
            make_box(LayoutBoxKind::TableWrapper, LayoutDisplay::Table, None),
            false,
        );
        let root = world.root();
        let caption = world.allocate(make_box(
            LayoutBoxKind::TableCaption,
            LayoutDisplay::TableCaption,
            Some(20.0),
        ));
        let group = world.allocate(make_box(
            LayoutBoxKind::TableRowGroup,
            LayoutDisplay::TableRowGroup,
            Some(80.0),
        ));
        world.boxes[root.index()].children = vec![caption, group];
        world.boxes[caption.index()].parent = Some(root);
        world.boxes[group.index()].parent = Some(root);
        let mut cells = Vec::new();
        for height in [10.0, 30.0] {
            let row = world.allocate(make_box(
                LayoutBoxKind::TableRow,
                LayoutDisplay::TableRow,
                None,
            ));
            let cell = world.allocate(make_box(
                LayoutBoxKind::TableCell,
                LayoutDisplay::TableCell,
                Some(height),
            ));
            world.boxes[group.index()].children.push(row);
            world.boxes[row.index()].children.push(cell);
            world.boxes[row.index()].parent = Some(group);
            world.boxes[cell.index()].parent = Some(row);
            cells.push(cell);
        }
        let mut prepared = crate::taffy_tree::prepare_world_layout(
            &mut world,
            crate::PaintViewport::new(800, 600, 1.0),
        );
        let inputs = LayoutInput {
            known_dimensions: Size {
                width: Some(200.0),
                height: None,
            },
            definite_dimensions: Size {
                width: Some(200.0),
                height: None,
            },
            parent_size: Size {
                width: Some(200.0),
                height: None,
            },
            parent_writing_mode: WritingMode::HorizontalTb,
            available_space: Size {
                width: AvailableSpace::Definite(200.0),
                height: AvailableSpace::MaxContent,
            },
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            axis: RequestedAxis::Both,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let cold = world.compute_child_layout(root.to_taffy(), inputs);
        assert_eq!(
            cold.size,
            Size {
                width: 200.0,
                height: 100.0
            }
        );
        for id in cells.iter().chain([&caption, &group]) {
            assert_eq!(world.boxes[id.index()].unrounded_layout.size, Size::ZERO);
        }
        assert_eq!(cold.first_baselines.y, Some(40.0));
        assert_eq!(cold, world.compute_child_layout(root.to_taffy(), inputs));
        assert_eq!(
            world.boxes[root.index()]
                .table_measure_cache
                .as_ref()
                .unwrap()
                .computations,
            1
        );
        let final_layout = world.compute_child_layout(
            root.to_taffy(),
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                ..inputs
            },
        );
        let warm = world.compute_child_layout(root.to_taffy(), inputs);
        assert_eq!(cold.size, final_layout.size);
        assert_eq!(cold, warm);
        assert_eq!(
            world.boxes[group.index()].unrounded_layout.size.height,
            80.0
        );
        assert_eq!(
            world.boxes[cells[0].index()].unrounded_layout.size.height,
            20.0
        );
        assert_eq!(
            world.boxes[cells[1].index()].unrounded_layout.size.height,
            60.0
        );
        world.boxes[group.index()].style.taffy.size.height = Dimension::length(120.0);
        prepared.invalidate_scrollbar_feedback(&mut world, &[group], false);
        let changed = world.compute_child_layout(root.to_taffy(), inputs);
        assert_eq!(changed.size.height, 140.0);
        assert_eq!(changed.first_baselines.y, Some(50.0));
        assert_eq!(
            world.boxes[root.index()]
                .table_measure_cache
                .as_ref()
                .unwrap()
                .computations,
            2
        );
    }

    #[test]
    fn indexed_placement_clips_spans_at_group_boundaries_including_ungrouped_rows() {
        let a = LayoutBoxId::from_index(20);
        let b = LayoutBoxId::from_index(21);
        let groups = [Some(a), Some(a), Some(a), None, None, Some(b), None];
        let counts = [1, 1, 0, 1, 1, 1, 1];
        let spans = [(2, 0), (1, 99), (1, 0), (1, 1), (1, 99), (1, 0)];
        let mut rows = Vec::new();
        let mut cells = Vec::new();
        for (index, (&group, &cell_count)) in groups.iter().zip(&counts).enumerate() {
            rows.push(TableRow {
                id: LayoutBoxId::from_index(index),
                group,
                index,
                grid_index: index,
                start_cell_index: cells.len(),
                cell_count,
                section_end: 0,
            });
            for _ in 0..cell_count {
                let (column_span, row_span) = spans[cells.len()];
                cells.push(TableCell {
                    id: LayoutBoxId::from_index(30 + cells.len()),
                    style: Style::default(),
                    row: index,
                    column: 0,
                    row_span,
                    column_span,
                    block_layout: None,
                });
            }
        }
        index_row_sections(&mut rows);
        assert_eq!(
            rows.iter().map(|row| row.section_end).collect::<Vec<_>>(),
            [3, 3, 3, 5, 5, 6, 7]
        );
        let mut columns = 0;
        place_table_cells(&mut cells, &rows, &mut columns);
        assert_eq!(columns, 3);
        assert_eq!(
            cells
                .iter()
                .map(|cell| (cell.column, cell.row_span))
                .collect::<Vec<_>>(),
            [(0, 3), (2, 2), (0, 2), (1, 1), (0, 1), (0, 1)]
        );
    }

    #[test]
    fn nested_table_measurements_reuse_baselines_with_bounded_work() {
        use crate::{LayoutDisplay, PaintColor, ResolvedLayoutStyle};

        let make_box = |kind, display, height| {
            LayoutWorld::<usize>::new_box(
                None,
                None,
                None,
                "nested-table-cache".into(),
                None,
                None,
                None,
                kind,
                ResolvedLayoutStyle::synthetic(
                    display,
                    Style {
                        size: Size {
                            width: Dimension::length(100.0),
                            height,
                        },
                        ..Style::default()
                    },
                    PaintColor::TRANSPARENT,
                ),
                None,
            )
        };
        for depth in [2, 4, 8, 16] {
            let mut world = LayoutWorld::new(
                make_box(
                    LayoutBoxKind::TableWrapper,
                    LayoutDisplay::Table,
                    Dimension::auto(),
                ),
                false,
            );
            let root = world.root();
            let mut table = root;
            for level in 0..depth {
                let row = world.allocate(make_box(
                    LayoutBoxKind::TableRow,
                    LayoutDisplay::TableRow,
                    Dimension::auto(),
                ));
                let cell = world.allocate(make_box(
                    LayoutBoxKind::TableCell,
                    LayoutDisplay::TableCell,
                    Dimension::auto(),
                ));
                let nested = level + 1 < depth;
                let child = world.allocate(make_box(
                    if nested {
                        LayoutBoxKind::TableWrapper
                    } else {
                        LayoutBoxKind::PrincipalBlock
                    },
                    if nested {
                        LayoutDisplay::Table
                    } else {
                        LayoutDisplay::Block
                    },
                    if nested {
                        Dimension::auto()
                    } else {
                        Dimension::length(10.0)
                    },
                ));
                for (parent, child) in [(table, row), (row, cell), (cell, child)] {
                    world.boxes[parent.index()].children.push(child);
                    world.boxes[child.index()].parent = Some(parent);
                }
                table = child;
            }
            crate::taffy_tree::prepare_world_layout(
                &mut world,
                crate::PaintViewport::new(800, 600, 1.0),
            );
            let inputs = LayoutInput {
                run_mode: RunMode::ComputeSize,
                known_dimensions: Size {
                    width: Some(100.0),
                    height: None,
                },
                ..LayoutInput::HIDDEN
            };
            let before: Vec<_> = world.boxes.iter().map(|b| b.unrounded_layout).collect();
            let cold = world.compute_child_layout(root.to_taffy(), inputs);
            assert_eq!(
                world
                    .boxes
                    .iter()
                    .map(|b| b.unrounded_layout)
                    .collect::<Vec<_>>(),
                before,
                "complete baseline measurement must not publish numeric layouts"
            );
            assert_eq!(
                cold.size,
                Size {
                    width: 100.0,
                    height: 10.0
                }
            );
            assert_eq!(cold.first_baselines.y, Some(10.0));
            let computations = |world: &LayoutWorld<usize>| {
                world
                    .boxes
                    .iter()
                    .filter_map(|b| b.table_measure_cache.as_ref())
                    .map(|c| c.computations)
                    .sum::<usize>()
            };
            let measured = computations(&world);
            assert_eq!(
                world
                    .boxes
                    .iter()
                    .filter(|b| b.table_measure_cache.is_some())
                    .count(),
                depth
            );
            assert!(
                measured <= depth * 8,
                "depth={depth}, table measurements={measured}"
            );
            assert_eq!(cold, world.compute_child_layout(root.to_taffy(), inputs));
            assert_eq!(
                computations(&world),
                measured,
                "warm measurement must reuse the complete result"
            );
            eprintln!(
                "nested tables: depth={depth}, cold measurements={measured}, warm measurements=0"
            );
        }
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
}
