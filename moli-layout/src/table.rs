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
    AvailableSpace, CacheTree, DetailedGridInfo, Dimension, Display, GridAutoFlow,
    IntrinsicSizeResult, Layout, LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree,
    Line, LogicalSize, MaybeResolve, NodeId, Point, Rect, RequestedAxis, ResolveOrZero, RunMode,
    Size, SizingMode, SizingPurpose, Style, TraversePartialTree, TraverseTree, compute_grid_layout,
    style_helpers,
};

use crate::{LayoutBoxId, LayoutBoxKind, LayoutWorld, style::resolve_stylo_calc_value};

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
    layout_mode: TableLayoutMode,
    inline_border_spacing: f32,
    outer_border_spacing: Size<f32>,
    writing_mode: taffy::WritingMode,
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
    context.collect_cell_inline_constraints(world);
    let grid_inputs = context.resolve_column_tracks(inputs);
    let mut output = {
        let mut wrapper = TableTreeWrapper {
            world,
            context: &mut context,
        };
        compute_grid_layout(&mut wrapper, NodeId::from(0usize), grid_inputs)
    };

    if inputs.run_mode == RunMode::PerformLayout {
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
        let top_height = layout_captions(world, &top_captions, output.size, writing_mode, 0.0);
        shift_grid_children(world, &context.cells, top_height);
        let bottom_height = layout_captions(
            world,
            &bottom_captions,
            output.size,
            writing_mode,
            top_height + output.size.height,
        );
        apply_structural_layout(world, root, &context, inputs, top_height);
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
    max_columns = max_columns.max(column_tracks.len()).max(1);
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
        layout_mode,
        inline_border_spacing: spacing.width,
        outer_border_spacing: spacing,
        writing_mode,
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
        let spacing = self.outer_border_spacing;
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
        let used_inline_size = self.resolve_used_inline_size(
            inputs,
            grid_min_max.min,
            grid_min_max.max,
            inline_insets,
        );
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
            // Automatic tables cannot make a column narrower than its outer
            // min-content measure, even when a containing block supplies a
            // smaller known or preferred size.
            used = used.max(grid_min);
        }
        if let Some(max_size) = max_size {
            used = used.min(max_size);
        }
        if let Some(min_size) = min_size {
            used = used.max(min_size);
        }
        used.max(inline_insets)
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
                let mut cell_style = world.boxes[cell.index()].style.taffy.clone();
                cell_style.margin = Rect::ZERO.map(style_helpers::length);
                cells.push(TableCell {
                    id: cell,
                    style: cell_style,
                    row: row_index,
                    column: 0,
                    row_span,
                    column_span,
                });
            }
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

fn clear_table_cell_inline_sizing(style: &mut Style<Atom>, writing_mode: taffy::WritingMode) {
    set_physical_inline_dimension(writing_mode, &mut style.size, Dimension::auto());
    set_physical_inline_dimension(writing_mode, &mut style.min_size, Dimension::auto());
    set_physical_inline_dimension(writing_mode, &mut style.max_size, Dimension::auto());
}

fn normalize_table_cell_block_sizing(style: &mut Style<Atom>, writing_mode: taffy::WritingMode) {
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

fn layout_captions<N>(
    world: &mut LayoutWorld<N>,
    captions: &[LayoutBoxId],
    containing_size: Size<f32>,
    parent_writing_mode: taffy::WritingMode,
    mut y: f32,
) -> f32
where
    N: Copy + Debug + Eq + Hash,
{
    let start = y;
    let width = containing_size.width;
    let percentage_basis = parent_writing_mode.to_logical(containing_size).inline_size;
    for (order, caption) in captions.iter().copied().enumerate() {
        let style = world.boxes[caption.index()].style.taffy.clone();
        let margin = style
            .margin
            .resolve_or_zero(Some(percentage_basis), resolve_stylo_calc_value);
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
            parent_size: containing_size.map(Some),
            parent_writing_mode,
            available_space: Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::MaxContent,
            },
            sizing_mode: SizingMode::InherentSize,
            sizing_purpose: SizingPurpose::Layout,
            run_mode: RunMode::PerformLayout,
            axis: taffy::RequestedAxis::Both,
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let output = world.compute_child_layout(caption.to_taffy(), inputs);
        set_box_layout(
            world,
            caption,
            Point { x: margin.left, y },
            output,
            order,
            Some(percentage_basis),
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
    inputs: LayoutInput,
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
    let structural_percentage_basis = context
        .writing_mode
        .to_logical(Size {
            width: content_width,
            height: content_height,
        })
        .inline_size;
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
        set_structural_rect(
            world,
            row.id,
            origin.x,
            y,
            content_width,
            height,
            structural_percentage_basis,
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
            let y = row_starts.get(start).copied().unwrap_or(origin.y);
            let height =
                track_range_extent(&detailed.rows.sizes, &detailed.rows.gutters, start, end);
            set_structural_rect(
                world,
                group,
                origin.x,
                y,
                content_width,
                height,
                structural_percentage_basis,
            );
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
        set_structural_rect(
            world,
            column.id,
            x,
            origin.y,
            width,
            content_height,
            structural_percentage_basis,
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
            let x = column_starts.get(start).copied().unwrap_or(origin.x);
            let width = track_range_extent(
                &detailed.columns.sizes,
                &detailed.columns.gutters,
                start,
                end,
            );
            set_structural_rect(
                world,
                group,
                x,
                origin.y,
                width,
                content_height,
                structural_percentage_basis,
            );
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

fn set_structural_rect<N>(
    world: &mut LayoutWorld<N>,
    id: LayoutBoxId,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    percentage_basis: f32,
) where
    N: Copy + Debug + Eq + Hash,
{
    let style = &world.boxes[id.index()].style.taffy;
    let padding = style
        .padding
        .resolve_or_zero(Some(percentage_basis), resolve_stylo_calc_value);
    let border = style
        .border
        .resolve_or_zero(Some(percentage_basis), resolve_stylo_calc_value);
    world.boxes[id.index()].unrounded_layout = Layout {
        order: 0,
        location: Point { x, y },
        size: Size { width, height },
        content_size: Size { width, height },
        scrollbar_size: Size::ZERO,
        border,
        padding,
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
    let margin = style
        .margin
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
        // zero, column sizing has consumed every applicable width constraint,
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
