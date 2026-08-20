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
    FontBaseline, GridAutoFlow, IntrinsicSizeResult, Layout, LayoutGridContainer, LayoutInput,
    LayoutOutput, LayoutPartialTree, Line, LogicalOffset, LogicalSize, MaybeResolve, NodeId, Point,
    Rect, RequestedAxis, ResolveOrZero, RunMode, Size, SizingMode, SizingPurpose, Style,
    TableCellLayoutInput, TraversePartialTree, TraverseTree, WritingDirection, compute_grid_layout,
    style_helpers,
};

use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutInlineAlignment, LayoutRect, LayoutWorld,
    style::resolve_stylo_calc_value,
};

mod collapsed_borders;
mod columns;

pub(crate) use collapsed_borders::CollapsedTableBorders;
use collapsed_borders::{prepare_collapsed_table_borders, set_collapsed_border_geometry};
use columns::{
    TableCellInlineConstraint, TableCellSpanConstraint, TableColumnConstraint, TableLayoutMode,
    apply_cell_constraints, compute_grid_inline_min_max, distribute_auto_columns,
    distribute_fixed_columns,
};

#[derive(Clone)]
struct TableCell {
    id: LayoutBoxId,
    style: Style<Atom>,
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
    has_in_flow_content: bool,
}

/// Baseline data gathered while a row's cells still have intrinsic block
/// sizes. CSS Tables resolves this before distributing any excess table
/// height, then feeds the shared ascent back into final cell layout.
#[derive(Clone, Copy, Debug, Default)]
struct TableRowBaselineMetrics {
    max_ascent: Option<f32>,
    max_descent: Option<f32>,
    fallback_descent: Option<f32>,
}

/// A sizing-only item in the pass-local Grid adapter.
///
/// CSS table rows must be at least the sum of the largest baseline ascent and
/// descent, even when those extrema come from different cells. Representing
/// that requirement as a Grid item preserves authored row percentages and the
/// existing excess-height distribution instead of replacing the row track.
struct TableRowBaselineStrut {
    style: Style<Atom>,
    size: Size<f32>,
}

#[derive(Clone, Copy)]
struct TableRow {
    id: LayoutBoxId,
    group: Option<LayoutBoxId>,
    index: usize,
    track: taffy::TrackSizingFunction,
}

#[derive(Clone, Copy)]
struct TableColumn {
    id: LayoutBoxId,
    group: Option<LayoutBoxId>,
    start: usize,
    span: usize,
}

/// The single boundary where logical table-grid geometry becomes physical
/// fragment geometry.
///
/// Track sizing, table-part ranges, and collapsed-border conflicts stay in
/// logical row/column coordinates. Captions contribute a physical offset
/// outside the grid, so it is carried here and applied during projection.
#[derive(Clone, Copy, Debug)]
struct TableGridCoordinateSpace {
    writing_direction: WritingDirection,
    outer_size: Size<f32>,
    physical_offset: Point<f32>,
}

impl TableGridCoordinateSpace {
    const fn new(
        writing_direction: WritingDirection,
        outer_size: Size<f32>,
        physical_offset: Point<f32>,
    ) -> Self {
        Self {
            writing_direction,
            outer_size,
            physical_offset,
        }
    }

    fn physical_rect(
        self,
        logical_offset: LogicalOffset<f32>,
        logical_size: LogicalSize<f32>,
    ) -> LayoutRect {
        let physical_size = self.writing_direction.mode.to_physical(logical_size);
        let location = self
            .writing_direction
            .converter(self.outer_size)
            .to_physical_point(logical_offset, physical_size);
        LayoutRect::new(
            self.physical_offset.x + location.x,
            self.physical_offset.y + location.y,
            physical_size.width,
            physical_size.height,
        )
    }
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
    caption_min_inline_size: f32,
    top_caption_height: f32,
    bottom_caption_height: f32,
    detailed: Option<DetailedGridInfo>,
    collapsed_borders: bool,
    column_count: usize,
    column_constraints: Vec<TableColumnConstraint>,
    column_sizes: Vec<f32>,
    row_baseline_metrics: Vec<TableRowBaselineMetrics>,
    row_baseline_struts: Vec<TableRowBaselineStrut>,
    row_baselines: Vec<f32>,
    layout_mode: TableLayoutMode,
    inline_border_spacing: f32,
    outer_border_spacing: Size<f32>,
    writing_mode: taffy::WritingMode,
    writing_direction: WritingDirection,
    font_baseline: FontBaseline,
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
        // CSS tables cannot shrink below GRID_MIN, even when an authored
        // width/max-width is smaller. Project that table-specific lower bound
        // through Taffy's generic parent sizing as `min-content`; retain the
        // authored minimum separately for the table formatter's own sizing.
        let table = &mut world.boxes[root.index()];
        let writing_mode = table.style.writing_mode();
        if table.table_authored_min_inline_size.is_none() {
            table.table_authored_min_inline_size = Some(
                writing_mode
                    .to_logical(table.style.taffy.min_size)
                    .inline_size,
            );
        }
        set_physical_inline_dimension(
            writing_mode,
            &mut table.style.taffy.min_size,
            Dimension::min_content(),
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
    context.resolve_used_table_box(inputs);
    context.collect_caption_inline_constraints(world);
    context.collect_cell_inline_constraints(world);
    let mut grid_inputs = context.resolve_column_tracks(inputs);
    let block_size_is_requested = inputs.run_mode == RunMode::PerformLayout
        || inputs.axis.contains(context.writing_mode.block_axis());
    if block_size_is_requested {
        context.collect_caption_block_sizes(world, grid_inputs);
        grid_inputs = context.row_grid_inputs(grid_inputs);
        context.measure_row_baselines(world);
        context.materialize_row_baseline_struts();
    }
    // CSS Tables determines ROWMIN before applying the table-root's block
    // sizing properties. The authored height is then a minimum target for the
    // row grid, and max-height is not allowed to shrink the table below
    // ROWMIN. The virtual Grid adapter therefore needs both values: a
    // content-size pass with an indefinite block axis, followed by the normal
    // constrained pass that distributes any excess height among rows.
    normalize_table_block_intrinsic_sizing(&mut context.style, context.writing_mode);
    let natural_grid_block_size = block_size_is_requested.then(|| {
        let natural_inputs = table_intrinsic_block_inputs(
            grid_inputs,
            context.writing_mode,
            inputs.run_mode == RunMode::PerformLayout,
        );
        let mut wrapper = TableTreeWrapper {
            world,
            context: &mut context,
        };
        let natural_output =
            compute_grid_layout(&mut wrapper, NodeId::from(0usize), natural_inputs);
        context.resolve_natural_row_baselines();
        context
            .writing_mode
            .to_logical(natural_output.size)
            .block_size
    });
    let mut output = {
        let mut wrapper = TableTreeWrapper {
            world,
            context: &mut context,
        };
        compute_grid_layout(&mut wrapper, NodeId::from(0usize), grid_inputs)
    };
    let (first_baselines, last_baselines) =
        table_row_baseline_sets(&context, grid_inputs, output.size);
    output.first_baselines = first_baselines;
    output.last_baselines = last_baselines;

    // A parent-owned known block size is already the final used size for this
    // box. Otherwise the table owns its synthesized block size and CSS Tables
    // requires ROWMIN to encompass the result after ordinary height/min/max
    // resolution. Grid tracks retain their intrinsic bases when the
    // constrained container is smaller, so enlarging the table border box to
    // ROWMIN does not require a third row-layout pass.
    let known_grid_block_size = context
        .writing_mode
        .to_logical(grid_inputs.known_dimensions)
        .block_size;
    if let (None, Some(natural_grid_block_size)) = (known_grid_block_size, natural_grid_block_size)
    {
        let mut logical_output_size = context.writing_mode.to_logical(output.size);
        logical_output_size.block_size =
            logical_output_size.block_size.max(natural_grid_block_size);
        output.size = context.writing_mode.to_physical(logical_output_size);
    }

    if inputs.run_mode == RunMode::PerformLayout {
        let grid_outer_size = output.size;
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
        let writing_mode = world.boxes[root.index()].style.writing_mode();
        let top_height = compute_caption_stack(
            world,
            &top_captions,
            output.size,
            writing_mode,
            0.0,
            RunMode::PerformLayout,
        );
        shift_grid_children(world, &context.cells, top_height);
        let bottom_height = compute_caption_stack(
            world,
            &bottom_captions,
            output.size,
            writing_mode,
            top_height + output.size.height,
            RunMode::PerformLayout,
        );
        apply_structural_layout(world, root, &context, inputs, grid_outer_size, top_height);
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
    } else {
        let caption_height = context.top_caption_height + context.bottom_caption_height;
        if let Some(first_baseline) = &mut output.first_baselines.y {
            *first_baseline += context.top_caption_height;
        }
        if let Some(last_baseline) = &mut output.last_baselines.y {
            *last_baseline += context.top_caption_height;
        }
        output.size.height += caption_height;
        output.content_size.height += caption_height;
        output.content_size.width = output.content_size.width.min(output.size.width);
        output.content_size.height = output.content_size.height.min(output.size.height);
    }
    output
}

/// Build the first-pass constraint space used to establish a table's ROWMIN.
///
/// The final column width remains definite because cell block contributions
/// can depend on wrapping. The block axis and its percentage basis are
/// deliberately indefinite, matching Blink's initial table block-size pass.
fn table_intrinsic_block_inputs(
    mut inputs: LayoutInput,
    writing_mode: taffy::WritingMode,
    retain_fragment_geometry: bool,
) -> LayoutInput {
    let mut known_size = writing_mode.to_logical(inputs.known_dimensions);
    known_size.block_size = None;
    inputs.known_dimensions = writing_mode.to_physical(known_size);

    let mut definite_size = writing_mode.to_logical(inputs.definite_dimensions);
    definite_size.block_size = None;
    inputs.definite_dimensions = writing_mode.to_physical(definite_size);

    let mut parent_size = writing_mode.to_logical(inputs.parent_size);
    parent_size.block_size = None;
    inputs.parent_size = writing_mode.to_physical(parent_size);

    let mut available_size = writing_mode.to_logical(inputs.available_space);
    available_size.block_size = AvailableSpace::MaxContent;
    inputs.available_space = writing_mode.to_physical(available_size);

    // Final table layout needs the natural row tracks as well as their total
    // size. Ask Grid to retain fragment geometry only for that final pass;
    // intrinsic probes stay side-effect free and consume the size alone.
    inputs.run_mode = if retain_fragment_geometry {
        RunMode::PerformLayout
    } else {
        RunMode::ComputeSize
    };
    inputs.sizing_mode = SizingMode::ContentSize;
    inputs.sizing_purpose = SizingPurpose::IntrinsicContribution;
    inputs.axis = RequestedAxis::from(writing_mode.block_axis());
    inputs.block_auto_behavior = AutoSizeBehavior::FitContent;
    inputs.block_margins_are_collapsible = Line::FALSE;
    inputs
}

/// Remove block-axis intrinsic keywords from the virtual Grid's constrained
/// pass while retaining them on the authored table style.
///
/// Blink resolves these keywords through a table block-size callback during
/// the initial indefinite pass. At that point the callback is indefinite, so
/// preferred/minimum keywords behave as `auto` and maximum keywords as
/// `none`. ROWMIN, measured separately above, remains the real lower bound.
fn normalize_table_block_intrinsic_sizing(
    style: &mut Style<Atom>,
    writing_mode: taffy::WritingMode,
) {
    let mut size = writing_mode.to_logical(style.size);
    let mut min_size = writing_mode.to_logical(style.min_size);
    let mut max_size = writing_mode.to_logical(style.max_size);
    if size.block_size.is_intrinsic() {
        size.block_size = Dimension::auto();
    }
    if min_size.block_size.is_intrinsic() {
        min_size.block_size = Dimension::auto();
    }
    if max_size.block_size.is_intrinsic() {
        max_size.block_size = Dimension::auto();
    }
    style.size = writing_mode.to_physical(size);
    style.min_size = writing_mode.to_physical(min_size);
    style.max_size = writing_mode.to_physical(max_size);
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
    if let Some(authored_min_inline_size) = world.boxes[root.index()].table_authored_min_inline_size
    {
        set_physical_inline_dimension(
            root_style.writing_mode(),
            &mut style.min_size,
            authored_min_inline_size,
        );
    }
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
    let layout_mode = if root_style.table_layout_is_fixed() {
        TableLayoutMode::Fixed
    } else {
        TableLayoutMode::Automatic
    };
    let writing_mode = root_style.writing_mode();
    for column in grouped_children.columns.iter().copied() {
        collect_columns(
            world,
            column,
            None,
            &mut columns,
            &mut column_tracks,
            writing_mode,
        );
    }
    for section in grouped_children.sections() {
        collect_rows(world, section, None, &mut rows, &mut cells, writing_mode);
    }
    place_table_cells(&mut cells, &rows, &mut max_columns);
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
    max_columns = max_columns.max(column_tracks.len());
    column_tracks.resize(max_columns, TableColumnConstraint::auto());
    // The real tracks are materialized after all cell constraints have been
    // collected. This placeholder only keeps the virtual Grid shape explicit
    // while the CSS table algorithm owns sizing.
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
    let row_count = rows.len();
    TableContext {
        style,
        cells,
        rows,
        columns,
        captions: grouped_children.captions,
        caption_min_inline_size: 0.0,
        top_caption_height: 0.0,
        bottom_caption_height: 0.0,
        detailed: None,
        collapsed_borders: collapsed,
        column_count: max_columns,
        column_constraints: column_tracks,
        column_sizes: Vec::new(),
        row_baseline_metrics: vec![TableRowBaselineMetrics::default(); row_count],
        row_baseline_struts: Vec::new(),
        row_baselines: Vec::new(),
        layout_mode,
        inline_border_spacing: spacing.width,
        outer_border_spacing: spacing,
        writing_mode,
        writing_direction: root_style.writing_direction(),
        font_baseline: root_style.font_baseline(),
    }
}

impl TableContext {
    /// Materialize the numeric Grid adapter only after the containing-block
    /// constraint space is available. The authored table padding remains
    /// unresolved in `build_table_context`; resolving it there would turn
    /// every percentage into zero before the table knows its percentage basis.
    fn resolve_used_table_box(&mut self, inputs: LayoutInput) {
        if self.collapsed_borders {
            return;
        }

        let percentage_basis = inputs
            .constraint_space(self.writing_mode)
            .margin_padding_percentage_basis();
        let padding = self
            .style
            .padding
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        // Border spacing surrounds real table tracks: inline spacing requires
        // a column, and block spacing additionally requires a row. Keeping
        // zero tracks intact makes authored border/padding/caption sizes
        // distinguishable from spacing around a real table grid.
        let spacing = Size {
            width: if self.column_count == 0 {
                0.0
            } else {
                self.outer_border_spacing.width
            },
            height: if self.column_count == 0 || self.rows.is_empty() {
                0.0
            } else {
                self.outer_border_spacing.height
            },
        };
        let spacing_padding = if self.writing_mode.is_horizontal() {
            Rect {
                left: spacing.width,
                right: spacing.width,
                top: spacing.height,
                bottom: spacing.height,
            }
        } else {
            Rect {
                left: spacing.height,
                right: spacing.height,
                top: spacing.width,
                bottom: spacing.width,
            }
        };
        self.style.padding = Rect {
            left: style_helpers::length(padding.left + spacing_padding.left),
            right: style_helpers::length(padding.right + spacing_padding.right),
            top: style_helpers::length(padding.top + spacing_padding.top),
            bottom: style_helpers::length(padding.bottom + spacing_padding.bottom),
        };
    }

    /// Measure the minimum outer inline contribution of every caption.
    ///
    /// A caption is a block box in the table wrapper, not a grid cell. Its
    /// minimum contribution nevertheless places a lower bound on the table's
    /// used inline size. Keep that constraint beside the column constraints
    /// instead of manufacturing a grid track for captions; this mirrors
    /// Blink's `ComputeCaptionConstraint` / `ComputeAssignableTableInlineSize`
    /// split and leaves the CSS table box tree intact.
    fn collect_caption_inline_constraints<N>(&mut self, world: &mut LayoutWorld<N>)
    where
        N: Copy + Debug + Eq + Hash,
    {
        let inline_axis = self.writing_mode.inline_axis();
        let available_space = self.writing_mode.to_physical(LogicalSize {
            inline_size: AvailableSpace::MinContent,
            block_size: AvailableSpace::MaxContent,
        });
        let inputs = LayoutInput {
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size: Size::NONE,
            parent_writing_mode: self.writing_mode,
            available_space,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::IntrinsicContribution,
            run_mode: RunMode::ComputeSize,
            axis: RequestedAxis::from(inline_axis),
            inline_auto_behavior: AutoSizeBehavior::FitContent,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            block_margins_are_collapsible: Line::FALSE,
            table_cell: None,
        };

        self.caption_min_inline_size = self
            .captions
            .iter()
            .copied()
            .map(|caption| {
                let style = &world.boxes[caption.index()].style.taffy;
                // Percentages are cyclic while the table wrapper's intrinsic
                // inline size is being determined, and auto margins contribute
                // zero. `None` expresses both rules at the sizing boundary.
                let margin = style.margin.resolve_or_zero(None, resolve_stylo_calc_value);
                let inline_margin = physical_inline_sum(self.writing_mode, margin);
                let contribution = world
                    .compute_child_size(caption.to_taffy(), inputs)
                    .size
                    .get_abs(inline_axis);
                contribution + inline_margin
            })
            .fold(0.0, f32::max);
    }

    /// Measure caption stacks at the table's resolved inline size. Captions
    /// participate in the outer table wrapper, so a parent-owned known block
    /// size must be split between these stacks and the inner row grid.
    fn collect_caption_block_sizes<N>(&mut self, world: &mut LayoutWorld<N>, inputs: LayoutInput)
    where
        N: Copy + Debug + Eq + Hash,
    {
        let inline_size = self
            .writing_mode
            .to_logical(inputs.known_dimensions)
            .inline_size
            .unwrap_or(0.0);
        let containing_size = self.writing_mode.to_physical(LogicalSize {
            inline_size,
            block_size: 0.0,
        });
        let top = self
            .captions
            .iter()
            .copied()
            .filter(|caption| !world.boxes[caption.index()].style.caption_is_bottom())
            .collect::<Vec<_>>();
        let bottom = self
            .captions
            .iter()
            .copied()
            .filter(|caption| world.boxes[caption.index()].style.caption_is_bottom())
            .collect::<Vec<_>>();
        self.top_caption_height = compute_caption_stack(
            world,
            &top,
            containing_size,
            self.writing_mode,
            0.0,
            RunMode::ComputeSize,
        );
        self.bottom_caption_height = compute_caption_stack(
            world,
            &bottom,
            containing_size,
            self.writing_mode,
            0.0,
            RunMode::ComputeSize,
        );
    }

    fn row_grid_inputs(&self, mut inputs: LayoutInput) -> LayoutInput {
        let caption_height = self.top_caption_height + self.bottom_caption_height;
        let subtract_captions = |size: Size<Option<f32>>| {
            let mut logical = self.writing_mode.to_logical(size);
            logical.block_size = logical
                .block_size
                .map(|block_size| (block_size - caption_height).max(0.0));
            self.writing_mode.to_physical(logical)
        };
        inputs.known_dimensions = subtract_captions(inputs.known_dimensions);
        inputs.definite_dimensions = subtract_captions(inputs.definite_dimensions);
        inputs
    }

    /// Gather cell measures after the table tree is complete. Fixed layout
    /// consumes the first visual row; automatic layout consumes every row.
    fn collect_cell_inline_constraints<N>(&mut self, world: &mut LayoutWorld<N>)
    where
        N: Copy + Debug + Eq + Hash,
    {
        let mut cell_constraints: Vec<Option<TableCellInlineConstraint>> =
            vec![None; self.column_count];
        let mut cell_spans = Vec::new();
        for cell in &self.cells {
            if self.layout_mode.is_fixed() && cell.row != 0 {
                continue;
            }
            let constraint =
                table_cell_inline_constraint(world, cell.id, self.writing_mode, self.layout_mode);
            if cell.column_span == 1 {
                let Some(slot) = cell_constraints.get_mut(cell.column) else {
                    continue;
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
        let decoration_percentage_basis = inputs
            .constraint_space(self.writing_mode)
            .margin_padding_percentage_basis();
        let padding = self
            .style
            .padding
            .resolve_or_zero(decoration_percentage_basis, resolve_stylo_calc_value);
        let border = self
            .style
            .border
            .resolve_or_zero(decoration_percentage_basis, resolve_stylo_calc_value);
        let inline_insets = physical_inline_sum(self.writing_mode, padding)
            + physical_inline_sum(self.writing_mode, border);
        let internal_spacing =
            self.inline_border_spacing.max(0.0) * self.column_count.saturating_sub(1) as f32;
        let undistributable_space = inline_insets + internal_spacing;
        let grid_min_max = compute_grid_inline_min_max(
            &self.column_constraints,
            undistributable_space,
            self.layout_mode.is_fixed(),
        );
        // Captions live in the wrapper formatting context rather than the
        // column grid, but their minimum outer contribution constrains both
        // intrinsic table contributions and the final used table width.
        let table_min = grid_min_max.min.max(self.caption_min_inline_size);
        let table_max = grid_min_max.max.max(self.caption_min_inline_size);
        let used_inline_size =
            self.resolve_used_inline_size(inputs, table_min, table_max, inline_insets);
        let assignable_inline_size = (used_inline_size - undistributable_space).max(0.0);
        let column_sizes = if self.layout_mode.is_fixed() {
            distribute_fixed_columns(assignable_inline_size, &self.column_constraints)
        } else {
            distribute_auto_columns(
                assignable_inline_size,
                &self.column_constraints,
                /* treat_target_size_as_constrained */ true,
            )
        };
        self.column_sizes.clone_from(&column_sizes);
        self.style.grid_template_columns = column_sizes
            .into_iter()
            .map(|size| {
                let track: taffy::TrackSizingFunction = style_helpers::length(size);
                track.into()
            })
            .collect();

        // The adapter retains authored box-sizing in the block axis. Convert
        // the already-resolved border-box inline size back to the numeric value
        // expected by that box-sizing mode, then remove consumed min/max
        // constraints so Grid cannot re-run CSS table sizing.
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

        let mut grid_space = inputs.constraint_space(self.writing_mode);
        grid_space.known_size.inline_size = Some(used_inline_size);
        grid_space.definite_size.inline_size = Some(used_inline_size);
        grid_space.into_layout_input()
    }

    /// Measure cell fragments with their final column widths before row
    /// sizing distributes any excess table block-size.
    ///
    /// CSS Tables forms one shared baseline per row from the largest ascent
    /// and descent of its baseline-aligned cells. Cells that span later rows
    /// contribute their ascent to the starting row but no descent. The
    /// fallback descent is retained separately for rows without a baseline
    /// participant.
    fn measure_row_baselines<N>(&mut self, world: &mut LayoutWorld<N>)
    where
        N: Copy + Debug + Eq + Hash,
    {
        self.row_baseline_metrics
            .fill(TableRowBaselineMetrics::default());

        for cell_index in 0..self.cells.len() {
            let cell = &self.cells[cell_index];
            let inline_size = self.cell_inline_size(cell);
            let known_dimensions = self.writing_mode.to_physical(LogicalSize {
                inline_size: Some(inline_size),
                block_size: None,
            });
            let available_space = self.writing_mode.to_physical(LogicalSize {
                inline_size: AvailableSpace::Definite(inline_size),
                block_size: AvailableSpace::MaxContent,
            });
            let inputs = LayoutInput {
                known_dimensions,
                definite_dimensions: known_dimensions,
                parent_size: known_dimensions,
                parent_writing_mode: self.writing_mode,
                available_space,
                sizing_mode: SizingMode::InherentSize,
                sizing_purpose: SizingPurpose::Layout,
                run_mode: RunMode::ComputeSize,
                axis: RequestedAxis::Both,
                inline_auto_behavior: AutoSizeBehavior::StretchImplicit,
                block_auto_behavior: AutoSizeBehavior::FitContent,
                block_margins_are_collapsible: Line::FALSE,
                table_cell: Some(TableCellLayoutInput::MEASURE),
            };
            let output = {
                let mut wrapper = TableTreeWrapper {
                    world,
                    context: self,
                };
                wrapper.with_grid_cell_style(cell_index, |world, cell| {
                    world.compute_child_layout(cell.to_taffy(), inputs)
                })
            };

            let cell = &self.cells[cell_index];
            let row = cell.row;
            let block_size = self
                .writing_mode
                .to_logical(output.size)
                .block_size
                .max(0.0);
            let baseline =
                logical_block_baseline(output.first_baselines, output.size, self.writing_direction);
            let participates = cell.has_in_flow_content
                && cell
                    .style
                    .align_content
                    .is_some_and(|alignment| alignment == taffy::AlignContent::BASELINE);

            let percentage_basis = Some(inline_size);
            let padding = cell
                .style
                .padding
                .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
            let border = cell
                .style
                .border
                .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
            let fallback_descent = self
                .writing_direction
                .to_logical_box_strut(padding + border)
                .block_end;
            let metrics = &mut self.row_baseline_metrics[row];
            metrics.fallback_descent = Some(
                metrics
                    .fallback_descent
                    .map_or(fallback_descent, |current| current.min(fallback_descent)),
            );

            if participates && let Some(baseline) = baseline {
                let ascent = baseline.clamp(0.0, block_size);
                let descent = if cell.row_span > 1 {
                    0.0
                } else {
                    (block_size - ascent).max(0.0)
                };
                metrics.max_ascent = Some(
                    metrics
                        .max_ascent
                        .map_or(ascent, |current| current.max(ascent)),
                );
                metrics.max_descent = Some(
                    metrics
                        .max_descent
                        .map_or(descent, |current| current.max(descent)),
                );
            }
        }
    }

    /// Add one sizing-only Grid item for every row with a shared baseline.
    /// The item carries ROWMIN's `max ascent + max descent` requirement while
    /// leaving authored track functions and excess-height distribution intact.
    fn materialize_row_baseline_struts(&mut self) {
        self.row_baseline_struts.clear();
        for (row, metrics) in self.row_baseline_metrics.iter().copied().enumerate() {
            let Some(ascent) = metrics.max_ascent else {
                continue;
            };
            let block_size = ascent + metrics.max_descent.unwrap_or(0.0);
            let logical_size = LogicalSize {
                inline_size: 0.0,
                block_size,
            };
            let size = self.writing_mode.to_physical(logical_size);
            let mut style = Style::<Atom> {
                display: Display::Block,
                size: size.map(style_helpers::length),
                min_size: size.map(style_helpers::length),
                max_size: size.map(style_helpers::length),
                ..Style::default()
            };
            style.grid_column = Line {
                start: style_helpers::line(1),
                end: style_helpers::span(1),
            };
            style.grid_row = Line {
                start: style_helpers::line((row + 1).min(i16::MAX as usize) as i16),
                end: style_helpers::span(1),
            };
            self.row_baseline_struts
                .push(TableRowBaselineStrut { style, size });
        }
    }

    /// Resolve each row's baseline against its natural track size. Baseline
    /// rows retain their measured shared ascent; rows without participants
    /// synthesize a baseline at block-end minus the smallest cell end inset.
    fn resolve_natural_row_baselines(&mut self) {
        let Some(detailed) = self.detailed.as_ref() else {
            self.row_baselines.clear();
            return;
        };
        let (row_sizes, _) = tracks_in_logical_order(
            &detailed.rows.sizes,
            &detailed.rows.gutters,
            self.writing_direction.is_block_flow_reversed(),
        );
        self.row_baselines = self
            .row_baseline_metrics
            .iter()
            .enumerate()
            .map(|(row, metrics)| {
                if let Some(ascent) = metrics.max_ascent {
                    return ascent;
                }
                let row_size = row_sizes.get(row).copied().unwrap_or(0.0);
                metrics
                    .fallback_descent
                    .map_or(0.0, |descent| (row_size - descent).max(0.0))
            })
            .collect();
    }

    fn cell_inline_size(&self, cell: &TableCell) -> f32 {
        let end = cell
            .column
            .saturating_add(cell.column_span)
            .min(self.column_sizes.len());
        if cell.column >= end {
            return 0.0;
        }
        self.column_sizes[cell.column..end].iter().sum::<f32>()
            + self.inline_border_spacing.max(0.0) * (end - cell.column - 1) as f32
    }

    fn resolve_used_inline_size(
        &self,
        inputs: LayoutInput,
        grid_min: f32,
        grid_max: f32,
        inline_insets: f32,
    ) -> f32 {
        let space = inputs.constraint_space(self.writing_mode);
        let available = space.available_size.inline_size;
        let fit_content = || match available {
            AvailableSpace::Definite(value) => grid_min.max(value.max(0.0).min(grid_max)),
            AvailableSpace::MinContent => grid_min,
            AvailableSpace::MaxContent => grid_max,
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
                Some(grid_min)
            } else if dimension.is_max_content() {
                Some(grid_max)
            } else if dimension.is_fit_content() {
                Some(fit_content())
            } else if dimension.is_stretch() {
                match available {
                    AvailableSpace::Definite(value) => Some(value.max(0.0)),
                    AvailableSpace::MinContent => Some(grid_min),
                    AvailableSpace::MaxContent => Some(grid_max),
                }
            } else {
                dimension
                    .maybe_resolve(percentage_basis, resolve_stylo_calc_value)
                    .map(|size| size + box_sizing_adjustment)
            }
        };

        let authored_sizes_apply = inputs.sizing_mode == SizingMode::InherentSize;
        // The outer tree replaces the table's min-inline-size with
        // `min-content` so parent algorithms include GRID_MIN in their used
        // size. Preserve an authored minimum in that intrinsic contribution,
        // but deliberately do not let max-inline-size cap GRID_MIN.
        let is_intrinsic_min_contribution = inputs.sizing_purpose
            == SizingPurpose::IntrinsicContribution
            && matches!(available, AvailableSpace::MinContent);
        let preferred = authored_sizes_apply
            .then(|| resolve_dimension(logical_size.inline_size))
            .flatten();
        let min_size = (authored_sizes_apply || is_intrinsic_min_contribution)
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
        if let Some(max_size) = max_size {
            used = used.min(max_size);
        }
        if let Some(min_size) = min_size {
            used = used.max(min_size);
        }
        used.max(grid_min).max(inline_insets)
    }
}

fn collect_columns<N>(
    world: &LayoutWorld<N>,
    current: LayoutBoxId,
    group: Option<LayoutBoxId>,
    columns: &mut Vec<TableColumn>,
    tracks: &mut Vec<TableColumnConstraint>,
    writing_mode: taffy::WritingMode,
) where
    N: Copy + Debug + Eq + Hash,
{
    match world.boxes[current.index()].kind {
        LayoutBoxKind::TableColumnGroup => {
            let before = tracks.len();
            for child in world.boxes[current.index()].children.iter().copied() {
                collect_columns(world, child, Some(current), columns, tracks, writing_mode);
            }
            if tracks.len() == before {
                let span = table_data(world, current).span.max(1) as usize;
                let track = table_column_constraint(
                    &world.boxes[current.index()].style.taffy,
                    writing_mode,
                );
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
            let track =
                table_column_constraint(&world.boxes[current.index()].style.taffy, writing_mode);
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
    writing_mode: taffy::WritingMode,
) where
    N: Copy + Debug + Eq + Hash,
{
    match world.boxes[current.index()].kind {
        LayoutBoxKind::TableRowGroup
        | LayoutBoxKind::TableHeaderGroup
        | LayoutBoxKind::TableFooterGroup
        | LayoutBoxKind::AnonymousTableRowGroup => {
            for child in world.boxes[current.index()].children.iter().copied() {
                collect_rows(world, child, Some(current), rows, cells, writing_mode);
            }
        }
        LayoutBoxKind::TableRow | LayoutBoxKind::AnonymousTableRow => {
            let row_index = rows.len();
            rows.push(TableRow {
                id: current,
                group,
                index: row_index,
                track: minimum_dimension_track(
                    writing_mode
                        .to_logical(world.boxes[current.index()].style.taffy.size)
                        .block_size,
                ),
            });
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
                let resolved_style = &world.boxes[cell.index()].style;
                let mut cell_style = resolved_style.taffy.clone();
                if cell_style.align_content.is_none() {
                    cell_style.align_content = Some(table_cell_normal_content_alignment(
                        resolved_style.vertical_align().kind,
                    ));
                }
                cell_style.margin = Rect::ZERO.map(style_helpers::length);
                cells.push(TableCell {
                    id: cell,
                    style: cell_style,
                    row: row_index,
                    column: 0,
                    row_span,
                    column_span,
                    has_in_flow_content: table_cell_has_in_flow_content(world, cell),
                });
            }
        }
        _ => {}
    }
}

/// Whether the table cell produces any normal-flow fragment that can supply
/// or consume a row baseline. Out-of-flow-only cells deliberately return
/// false: CSS Tables leaves their static positions at block-start when there
/// is no in-flow alignment subject.
fn table_cell_has_in_flow_content<N>(world: &LayoutWorld<N>, cell: LayoutBoxId) -> bool
where
    N: Copy + Debug + Eq + Hash,
{
    world.boxes[cell.index()]
        .layout_children
        .iter()
        .any(|child| world.boxes[child.index()].style.taffy.position != taffy::Position::Absolute)
}

/// Resolve table-cell `align-content: normal` through legacy
/// `vertical-align`, as required by CSS Box Alignment.
///
/// Explicit `align-content` is retained before this function is called.
/// Baseline-class values remain typed baseline requests rather than being
/// collapsed to a positional fallback. Positional values reuse block content
/// alignment so in-flow fragments and out-of-flow static-position candidates
/// move as one group.
fn table_cell_normal_content_alignment(
    vertical_align: LayoutInlineAlignment,
) -> taffy::AlignContent {
    match vertical_align {
        LayoutInlineAlignment::Top => taffy::AlignContent::START,
        LayoutInlineAlignment::Middle => taffy::AlignContent::SAFE_CENTER,
        LayoutInlineAlignment::Bottom => taffy::AlignContent::SAFE_END,
        LayoutInlineAlignment::Baseline
        | LayoutInlineAlignment::TextTop
        | LayoutInlineAlignment::TextBottom => taffy::AlignContent::BASELINE,
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

fn table_column_constraint(
    style: &Style<Atom>,
    writing_mode: taffy::WritingMode,
) -> TableColumnConstraint {
    let logical_size = writing_mode.to_logical(style.size);
    let logical_min_size = writing_mode.to_logical(style.min_size);
    let mut constraint = match logical_size.inline_size.tag() {
        taffy::CompactLength::LENGTH_TAG => {
            TableColumnConstraint::length(logical_size.inline_size.value())
        }
        taffy::CompactLength::PERCENT_TAG => {
            TableColumnConstraint::percent(logical_size.inline_size.value(), 0.0)
        }
        _ => TableColumnConstraint::explicit_auto(),
    };
    if logical_min_size.inline_size.tag() == taffy::CompactLength::LENGTH_TAG {
        constraint.min_inline_size = Some(logical_min_size.inline_size.value().max(0.0));
        if let Some(max_inline_size) = &mut constraint.max_inline_size {
            *max_inline_size = (*max_inline_size).max(constraint.min_inline_size.unwrap_or(0.0));
        }
    }
    constraint
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

fn table_cell_inline_constraint<N>(
    world: &mut LayoutWorld<N>,
    cell: LayoutBoxId,
    table_writing_mode: taffy::WritingMode,
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
    table_writing_mode: taffy::WritingMode,
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
        inline_auto_behavior: AutoSizeBehavior::FitContent,
        block_auto_behavior: AutoSizeBehavior::FitContent,
        block_margins_are_collapsible: Line::FALSE,
        table_cell: None,
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

fn clear_table_cell_inline_sizing(style: &mut Style<Atom>, writing_mode: taffy::WritingMode) {
    set_physical_inline_dimension(writing_mode, &mut style.size, Dimension::auto());
    set_physical_inline_dimension(writing_mode, &mut style.min_size, Dimension::auto());
    set_physical_inline_dimension(writing_mode, &mut style.max_size, Dimension::auto());
}

fn normalize_table_cell_block_sizing(style: &mut Style<Atom>, writing_mode: taffy::WritingMode) {
    // Table-cell layout ignores min/max block-size. An authored block-size is
    // instead a minimum contribution to its row (Blink's
    // `cell_css_block_size`), while the final row constraint stretches the
    // cell fragment separately.
    let mut size = writing_mode.to_logical(style.size);
    let mut min_size = writing_mode.to_logical(style.min_size);
    let mut max_size = writing_mode.to_logical(style.max_size);
    min_size.block_size = size.block_size;
    size.block_size = Dimension::auto();
    max_size.block_size = Dimension::auto();
    style.size = writing_mode.to_physical(size);
    style.min_size = writing_mode.to_physical(min_size);
    style.max_size = writing_mode.to_physical(max_size);
}

fn set_physical_inline_dimension(
    writing_mode: taffy::WritingMode,
    size: &mut Size<Dimension>,
    value: Dimension,
) {
    if writing_mode.is_horizontal() {
        size.width = value;
    } else {
        size.height = value;
    }
}

fn physical_inline_sum(writing_mode: taffy::WritingMode, rect: Rect<f32>) -> f32 {
    if writing_mode.is_horizontal() {
        rect.left + rect.right
    } else {
        rect.top + rect.bottom
    }
}

fn compute_caption_stack<N>(
    world: &mut LayoutWorld<N>,
    captions: &[LayoutBoxId],
    containing_size: Size<f32>,
    parent_writing_mode: taffy::WritingMode,
    mut y: f32,
    run_mode: RunMode,
) -> f32
where
    N: Copy + Debug + Eq + Hash,
{
    let start = y;
    let width = containing_size.width;
    let percentage_basis = parent_writing_mode.to_logical(containing_size).inline_size;
    for (order, caption) in captions.iter().copied().enumerate() {
        let style = world.boxes[caption.index()].style.taffy.clone();
        let mut margin = style
            .margin
            .resolve_or_zero(Some(percentage_basis), resolve_stylo_calc_value);
        y += margin.top;
        let available_inline_size =
            (percentage_basis - physical_inline_sum(parent_writing_mode, margin)).max(0.0);
        let parent_size = parent_writing_mode.to_physical(LogicalSize {
            inline_size: Some(percentage_basis),
            // CSS Tables treats caption percentage block sizes as auto: the
            // wrapper has no caption containing-block block size to expose.
            block_size: None,
        });
        let available_space = parent_writing_mode.to_physical(LogicalSize {
            inline_size: AvailableSpace::Definite(available_inline_size),
            block_size: AvailableSpace::MaxContent,
        });
        let inputs = LayoutInput {
            // The table supplies available margin-box space; the caption still
            // owns its authored width/min/max. An automatic width stretches,
            // while a specified width remains a real caption constraint.
            known_dimensions: Size::NONE,
            definite_dimensions: Size::NONE,
            parent_size,
            parent_writing_mode,
            available_space,
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode,
            axis: taffy::RequestedAxis::Both,
            inline_auto_behavior: AutoSizeBehavior::StretchImplicit,
            block_auto_behavior: AutoSizeBehavior::FitContent,
            block_margins_are_collapsible: Line::FALSE,
            table_cell: None,
        };
        let output = world.compute_child_layout(caption.to_taffy(), inputs);
        if run_mode == RunMode::PerformLayout && parent_writing_mode.is_horizontal() {
            let free_inline_space =
                (width - output.size.width - margin.left - margin.right).max(0.0);
            let auto_count = style.margin.left.is_auto() as u8 + style.margin.right.is_auto() as u8;
            if auto_count > 0 {
                let auto_margin = free_inline_space / f32::from(auto_count);
                if style.margin.left.is_auto() {
                    margin.left = auto_margin;
                }
                if style.margin.right.is_auto() {
                    margin.right = auto_margin;
                }
            }
        }
        if run_mode == RunMode::PerformLayout {
            set_box_layout(
                world,
                caption,
                Point { x: margin.left, y },
                output,
                order,
                Some(percentage_basis),
                margin,
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

/// Project a physical fragment baseline into a table's logical block axis.
fn logical_block_baseline(
    baseline: Point<Option<f32>>,
    fragment_size: Size<f32>,
    writing_direction: WritingDirection,
) -> Option<f32> {
    if writing_direction.mode.is_horizontal() {
        baseline.y
    } else {
        baseline.x.map(|offset| {
            if writing_direction.is_block_flow_reversed() {
                fragment_size.width - offset
            } else {
                offset
            }
        })
    }
}

/// Materialize one logical table baseline in physical fragment coordinates.
fn physical_baseline(
    baseline: Option<f32>,
    fragment_size: Size<f32>,
    writing_direction: WritingDirection,
) -> Point<Option<f32>> {
    if writing_direction.mode.is_horizontal() {
        Point {
            x: None,
            y: baseline,
        }
    } else {
        Point {
            x: baseline.map(|offset| {
                if writing_direction.is_block_flow_reversed() {
                    fragment_size.width - offset
                } else {
                    offset
                }
            }),
            y: None,
        }
    }
}

/// Export the first and last row baselines from the CSS table rather than the
/// generic Grid adapter. Captions remain outside this grid coordinate space
/// and are added by the table-wrapper path after they are laid out.
fn table_row_baseline_sets(
    context: &TableContext,
    inputs: LayoutInput,
    grid_outer_size: Size<f32>,
) -> (Point<Option<f32>>, Point<Option<f32>>) {
    let Some(detailed) = context.detailed.as_ref() else {
        return (Point::NONE, Point::NONE);
    };
    if context.rows.is_empty() || context.row_baselines.is_empty() {
        return (Point::NONE, Point::NONE);
    }

    let percentage_basis = inputs
        .constraint_space(context.writing_mode)
        .margin_padding_percentage_basis();
    let padding = context
        .style
        .padding
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let border = context
        .style
        .border
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let logical_padding = context.writing_direction.to_logical_box_strut(padding);
    let logical_border = context.writing_direction.to_logical_box_strut(border);
    let block_origin = logical_border.block_start + logical_padding.block_start;
    let (row_sizes, row_gutters) = tracks_in_logical_order(
        &detailed.rows.sizes,
        &detailed.rows.gutters,
        context.writing_direction.is_block_flow_reversed(),
    );
    let row_starts = track_starts(block_origin, &row_sizes, &row_gutters);
    let first = row_starts
        .first()
        .zip(context.row_baselines.first())
        .map(|(start, baseline)| start + baseline);
    let last_row = context.rows.len().saturating_sub(1);
    let last = row_starts
        .get(last_row)
        .zip(context.row_baselines.get(last_row))
        .map(|(start, baseline)| start + baseline);

    (
        physical_baseline(first, grid_outer_size, context.writing_direction),
        physical_baseline(last, grid_outer_size, context.writing_direction),
    )
}

fn apply_structural_layout<N>(
    world: &mut LayoutWorld<N>,
    root: LayoutBoxId,
    context: &TableContext,
    inputs: LayoutInput,
    grid_outer_size: Size<f32>,
    top_offset: f32,
) where
    N: Copy + Debug + Eq + Hash,
{
    let root_style = &context.style;
    let root_percentage_basis = inputs
        .constraint_space(context.writing_mode)
        .margin_padding_percentage_basis();
    let padding = root_style
        .padding
        .resolve_or_zero(root_percentage_basis, resolve_stylo_calc_value);
    let border = root_style
        .border
        .resolve_or_zero(root_percentage_basis, resolve_stylo_calc_value);
    let Some(detailed) = context.detailed.as_ref() else {
        return;
    };
    let writing_direction = world.boxes[root.index()].style.writing_direction();
    let logical_padding = writing_direction.to_logical_box_strut(padding);
    let logical_border = writing_direction.to_logical_box_strut(border);
    let inline_origin = logical_border.inline_start + logical_padding.inline_start;
    let block_origin = logical_border.block_start + logical_padding.block_start;
    let (column_sizes, column_gutters) = tracks_in_logical_order(
        &detailed.columns.sizes,
        &detailed.columns.gutters,
        writing_direction.is_inline_flow_reversed(),
    );
    let (row_sizes, row_gutters) = tracks_in_logical_order(
        &detailed.rows.sizes,
        &detailed.rows.gutters,
        writing_direction.is_block_flow_reversed(),
    );
    let column_starts = track_starts(inline_origin, &column_sizes, &column_gutters);
    let row_starts = track_starts(block_origin, &row_sizes, &row_gutters);
    let content_inline_size = track_extent(&column_sizes, &column_gutters);
    let content_block_size = track_extent(&row_sizes, &row_gutters);
    let coordinate_space = TableGridCoordinateSpace::new(
        writing_direction,
        grid_outer_size,
        Point {
            x: 0.0,
            y: top_offset,
        },
    );
    if context.collapsed_borders {
        let mut row_lines = row_starts.clone();
        row_lines.push(block_origin + content_block_size);
        let mut column_lines = column_starts.clone();
        column_lines.push(inline_origin + content_inline_size);
        set_collapsed_border_geometry(world, root, &column_lines, &row_lines, coordinate_space);
    }

    for row in &context.rows {
        let block_start = row_starts.get(row.index).copied().unwrap_or(block_origin);
        let block_size = row_sizes.get(row.index).copied().unwrap_or(0.0);
        set_logical_structural_rect(
            world,
            row.id,
            coordinate_space,
            LogicalOffset {
                inline_offset: inline_origin,
                block_offset: block_start,
            },
            LogicalSize {
                inline_size: content_inline_size,
                block_size,
            },
        );
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
            let block_start = row_starts.get(start).copied().unwrap_or(block_origin);
            let block_size = track_range_extent(&row_sizes, &row_gutters, start, end);
            set_logical_structural_rect(
                world,
                group,
                coordinate_space,
                LogicalOffset {
                    inline_offset: inline_origin,
                    block_offset: block_start,
                },
                LogicalSize {
                    inline_size: content_inline_size,
                    block_size,
                },
            );
        }
    }
    for column in &context.columns {
        let inline_start = column_starts
            .get(column.start)
            .copied()
            .unwrap_or(inline_origin);
        let inline_size = track_range_extent(
            &column_sizes,
            &column_gutters,
            column.start,
            column.start.saturating_add(column.span),
        );
        set_logical_structural_rect(
            world,
            column.id,
            coordinate_space,
            LogicalOffset {
                inline_offset: inline_start,
                block_offset: block_origin,
            },
            LogicalSize {
                inline_size,
                block_size: content_block_size,
            },
        );
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
            let inline_start = column_starts.get(start).copied().unwrap_or(inline_origin);
            let inline_size = track_range_extent(&column_sizes, &column_gutters, start, end);
            set_logical_structural_rect(
                world,
                group,
                coordinate_space,
                LogicalOffset {
                    inline_offset: inline_start,
                    block_offset: block_origin,
                },
                LogicalSize {
                    inline_size,
                    block_size: content_block_size,
                },
            );
        }
    }

    // Keep the root in the numeric tree even for an empty table.
    let _ = root;
}

/// Reconstruct logical start-to-end order from Taffy's detailed tracks, which
/// are exposed in ascending physical coordinates.
fn tracks_in_logical_order(
    sizes: &[f32],
    gutters: &[f32],
    flow_reversed: bool,
) -> (Vec<f32>, Vec<f32>) {
    let mut sizes = sizes.to_vec();
    let mut gutters = gutters.to_vec();
    if flow_reversed {
        sizes.reverse();
        gutters.reverse();
    }
    (sizes, gutters)
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

fn set_logical_structural_rect<N>(
    world: &mut LayoutWorld<N>,
    id: LayoutBoxId,
    coordinate_space: TableGridCoordinateSpace,
    logical_offset: LogicalOffset<f32>,
    logical_size: LogicalSize<f32>,
) where
    N: Copy + Debug + Eq + Hash,
{
    set_structural_rect(
        world,
        id,
        coordinate_space.physical_rect(logical_offset, logical_size),
    );
}

fn set_structural_rect<N>(world: &mut LayoutWorld<N>, id: LayoutBoxId, rect: LayoutRect)
where
    N: Copy + Debug + Eq + Hash,
{
    // Rows, row groups, columns, and column groups expose structural geometry
    // but do not establish ordinary CSS padding or border areas. In the
    // separated model their borders are ignored; in the collapsed model they
    // participate only through the table-owned conflict grid. Keep authored
    // style intact for CSSOM and resolve their used fragment decorations to
    // zero here, at the table formatting boundary.
    world.boxes[id.index()].unrounded_layout = Layout {
        order: 0,
        location: Point {
            x: rect.x,
            y: rect.y,
        },
        size: Size {
            width: rect.width,
            height: rect.height,
        },
        content_size: Size {
            width: rect.width,
            height: rect.height,
        },
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
    percentage_basis: Option<f32>,
    margin: Rect<f32>,
) where
    N: Copy + Debug + Eq + Hash,
{
    let style = &world.boxes[id.index()].style.taffy;
    let padding = style
        .padding
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
    let border = style
        .border
        .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
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
    ///
    /// Taffy's cache key contains layout inputs, not style identity. Clear the
    /// cell cache on both sides of the swap so intrinsic measurements made
    /// from the authored style and final Grid queries cannot alias each other.
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

    fn virtual_child_count(&self) -> usize {
        self.context.cells.len() + self.context.row_baseline_struts.len()
    }

    fn strut_index(&self, node_id: NodeId) -> Option<usize> {
        usize::from(node_id).checked_sub(self.context.cells.len())
    }

    fn prepare_cell_layout_input(&self, cell_index: usize, mut inputs: LayoutInput) -> LayoutInput {
        let cell = &self.context.cells[cell_index];
        inputs.table_cell = Some(TableCellLayoutInput::MEASURE);
        let cell_writing_mode = self.world.boxes[cell.id.index()].style.writing_mode();
        if !cell_writing_mode.is_orthogonal_to(self.context.writing_mode)
            && cell.has_in_flow_content
            && cell
                .style
                .align_content
                .is_some_and(|alignment| alignment == taffy::AlignContent::BASELINE)
            && let Some(row_baseline) = self.context.row_baselines.get(cell.row).copied()
        {
            let alignment_baseline = if cell_writing_mode.is_block_flow_reversed()
                != self.context.writing_mode.is_block_flow_reversed()
            {
                cell_writing_mode
                    .to_logical(inputs.known_dimensions)
                    .block_size
                    .map_or(row_baseline, |block_size| block_size - row_baseline)
            } else {
                row_baseline
            };
            inputs.table_cell = Some(TableCellLayoutInput::aligned_to(alignment_baseline));
        }
        inputs
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
        VirtualChildIter(0..self.virtual_child_count())
    }

    fn child_count(&self, _parent_node_id: NodeId) -> usize {
        self.virtual_child_count()
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

    fn get_writing_mode(&self, _node_id: NodeId) -> taffy::WritingMode {
        // The table grid is virtual, so its inherited writing mode cannot be
        // recovered from a real LayoutBoxId. Keep Taffy's logical track
        // sizing and placement in the table root's coordinate system from
        // the start instead of repairing physical axes after layout.
        self.context.writing_mode
    }

    fn get_font_baseline(&self, _node_id: NodeId) -> FontBaseline {
        self.context.font_baseline
    }

    fn get_size_containment(&self, _node_id: NodeId) -> taffy::SizeContainment {
        // CSS table wrappers and internal table boxes are ineligible for size
        // containment. Child cells leave this virtual tree through LayoutWorld,
        // which applies their own used eligibility at that boundary.
        taffy::SizeContainment::NONE
    }

    fn resolve_calc_value(&self, value: *const (), basis: f32) -> f32 {
        resolve_stylo_calc_value(value, basis)
    }

    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        if let Some(cell) = self.context.cells.get(usize::from(node_id)) {
            self.world.boxes[cell.id.index()].unrounded_layout = *layout;
        }
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let cell_index = usize::from(node_id);
        if let Some(strut_index) = self.strut_index(node_id) {
            return LayoutOutput::from_outer_size(
                self.context.row_baseline_struts[strut_index].size,
            );
        }
        let inputs = self.prepare_cell_layout_input(cell_index, inputs);
        // The virtual table grid owns the used grid-item style: margins are
        // zero, column sizing has consumed every applicable width constraint,
        // and cell block size is a minimum contribution.
        self.with_grid_cell_style(cell_index, |world, cell| {
            world.compute_child_layout(cell.to_taffy(), inputs)
        })
    }

    fn compute_child_size(
        &mut self,
        node_id: NodeId,
        mut inputs: LayoutInput,
    ) -> IntrinsicSizeResult {
        let cell_index = usize::from(node_id);
        if let Some(strut_index) = self.strut_index(node_id) {
            return IntrinsicSizeResult::from_size(
                self.context.row_baseline_struts[strut_index].size,
            );
        }
        inputs.table_cell = Some(TableCellLayoutInput::MEASURE);
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
        let child_index = usize::from(child_node_id);
        if let Some(cell) = self.context.cells.get(child_index) {
            &cell.style
        } else {
            &self.context.row_baseline_struts[child_index - self.context.cells.len()].style
        }
    }

    fn set_detailed_grid_info(&mut self, _node_id: NodeId, detailed_grid_info: DetailedGridInfo) {
        self.context.detailed = Some(detailed_grid_info);
    }
}
