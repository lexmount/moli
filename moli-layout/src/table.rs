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
    AvailableSpace, DetailedGridInfo, Dimension, Display, GridAutoFlow, Layout,
    LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree, Line, MaybeMath,
    MaybeResolve, NodeId, Point, Rect, ResolveOrZero, RunMode, Size, SizingMode, SizingPurpose,
    Style, TraversePartialTree, TraverseTree, compute_grid_layout, style_helpers,
};

use crate::{LayoutBoxId, LayoutBoxKind, LayoutWorld, style::resolve_stylo_calc_value};

mod collapsed_borders;
mod columns;

pub(crate) use collapsed_borders::CollapsedTableBorders;
use collapsed_borders::{prepare_collapsed_table_borders, set_collapsed_border_geometry};
use columns::{
    TableCellInlineConstraint, TableCellSpanConstraint, TableColumnConstraint,
    distribute_fixed_cell_spans, distribute_fixed_columns, fixed_grid_min_inline_size,
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
    fixed_layout: bool,
    inline_border_spacing: f32,
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
    let context = build_table_context(world, root);
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
    context.resolve_column_tracks(inputs);
    let mut output = {
        let mut wrapper = TableTreeWrapper {
            world,
            context: &mut context,
        };
        compute_grid_layout(&mut wrapper, NodeId::from(0usize), inputs)
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
        let top_height = layout_captions(world, &top_captions, output.size.width, 0.0);
        shift_grid_children(world, &context.cells, top_height);
        let bottom_height = layout_captions(
            world,
            &bottom_captions,
            output.size.width,
            top_height + output.size.height,
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
    let mut cell_span_constraints = Vec::new();
    let fixed_layout = root_style.table_layout_is_fixed();
    for column in grouped_children.columns.iter().copied() {
        collect_columns(world, column, None, &mut columns, &mut column_tracks);
    }
    for section in grouped_children.sections() {
        collect_rows(world, section, None, &mut rows, &mut cells);
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
        if cell.row == 0 {
            let inline_constraint = table_cell_inline_constraint(&cell.style, fixed_layout);
            if cell.column_span == 1 {
                if cell.column >= column_tracks.len() {
                    column_tracks.resize(cell.column + 1, TableColumnConstraint::auto());
                }
                column_tracks[cell.column].encompass_first_row_cell(inline_constraint);
            } else if fixed_layout {
                cell_span_constraints.push(TableCellSpanConstraint {
                    start_column: cell.column,
                    span: cell.column_span,
                    cell: inline_constraint,
                });
            }
        }
        cell.style.size.width = Dimension::auto();
        // CSS table-cell `height` is a minimum contribution, while the used
        // border box still fills its row or rowspan. A definite grid-item
        // height would leave a rowspan cell at one-row height instead.
        if cell.style.min_size.height == Dimension::auto() {
            cell.style.min_size.height = cell.style.size.height;
        }
        cell.style.size.height = Dimension::auto();
    }
    max_columns = max_columns.max(column_tracks.len()).max(1);
    column_tracks.resize(max_columns, TableColumnConstraint::auto());
    if fixed_layout {
        distribute_fixed_cell_spans(
            &mut column_tracks,
            &mut cell_span_constraints,
            spacing.width,
        );
    }
    style.grid_template_columns = column_tracks
        .iter()
        .copied()
        .map(|track| track.intrinsic_grid_track().into())
        .collect();
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
        columns,
        captions: grouped_children.captions,
        detailed: None,
        collapsed_borders: collapsed,
        column_count: max_columns,
        column_constraints: column_tracks,
        fixed_layout,
        inline_border_spacing: spacing.width,
    }
}

impl TableContext {
    /// Resolve table column semantics before invoking the numeric Grid backend.
    /// Grid receives final lengths for a definite fixed-layout table and never
    /// participates in the table free-space distribution algorithm.
    fn resolve_column_tracks(&mut self, inputs: LayoutInput) {
        let Some(assignable_inline_size) = self.fixed_assignable_inline_size(inputs) else {
            return;
        };
        self.style.grid_template_columns =
            distribute_fixed_columns(assignable_inline_size, &self.column_constraints)
                .into_iter()
                .map(|size| {
                    let track: taffy::TrackSizingFunction = style_helpers::length(size);
                    track.into()
                })
                .collect();
    }

    fn fixed_assignable_inline_size(&self, inputs: LayoutInput) -> Option<f32> {
        if !self.fixed_layout {
            return None;
        }

        let percentage_basis = inputs.parent_size.width;
        let padding = self
            .style
            .padding
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let border = self
            .style
            .border
            .resolve_or_zero(percentage_basis, resolve_stylo_calc_value);
        let inline_insets = padding.left + padding.right + border.left + border.right;
        let to_border_box = |size: Option<f32>| {
            size.map(|size| {
                if self.style.box_sizing == taffy::BoxSizing::ContentBox {
                    size + inline_insets
                } else {
                    size.max(inline_insets)
                }
            })
        };
        let (preferred, min_size, max_size) = if inputs.sizing_mode == SizingMode::InherentSize {
            (
                to_border_box(
                    self.style
                        .size
                        .width
                        .maybe_resolve(percentage_basis, resolve_stylo_calc_value),
                ),
                to_border_box(
                    self.style
                        .min_size
                        .width
                        .maybe_resolve(percentage_basis, resolve_stylo_calc_value),
                ),
                to_border_box(
                    self.style
                        .max_size
                        .width
                        .maybe_resolve(percentage_basis, resolve_stylo_calc_value),
                ),
            )
        } else {
            (None, None, None)
        };
        let synthesized_border_box_size = preferred
            .maybe_clamp(min_size, max_size)
            .maybe_max(Some(inline_insets));
        let border_box_size = inputs
            .known_dimensions
            .width
            .or(synthesized_border_box_size)?;
        let internal_spacing =
            self.inline_border_spacing.max(0.0) * self.column_count.saturating_sub(1) as f32;

        Some((border_box_size - inline_insets - internal_spacing).max(0.0))
    }

    fn fixed_grid_min_border_box_size(&self) -> Option<f32> {
        if !self.fixed_layout {
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
            rows.push(TableRow {
                id: current,
                group,
                index: row_index,
                track: minimum_dimension_track(
                    world.boxes[current.index()].style.taffy.size.height,
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

fn table_cell_inline_constraint(style: &Style<Atom>, fixed: bool) -> TableCellInlineConstraint {
    match style.size.width.tag() {
        taffy::CompactLength::LENGTH_TAG => {
            let padding = style
                .padding
                .resolve_or_zero(None, resolve_stylo_calc_value);
            let border = style.border.resolve_or_zero(None, resolve_stylo_calc_value);
            let border_padding = padding.left + padding.right + border.left + border.right;
            let outer_width = if style.box_sizing == taffy::BoxSizing::ContentBox {
                style.size.width.value() + border_padding
            } else {
                style.size.width.value().max(border_padding)
            };
            TableCellInlineConstraint::length(outer_width)
        }
        taffy::CompactLength::PERCENT_TAG if fixed => {
            let border_padding = if style.box_sizing == taffy::BoxSizing::ContentBox {
                let padding = style
                    .padding
                    .resolve_or_zero(None, resolve_stylo_calc_value);
                let border = style.border.resolve_or_zero(None, resolve_stylo_calc_value);
                padding.left + padding.right + border.left + border.right
            } else {
                0.0
            };
            TableCellInlineConstraint::percent(style.size.width.value(), border_padding)
        }
        _ => TableCellInlineConstraint::auto(),
    }
}

fn layout_captions<N>(
    world: &mut LayoutWorld<N>,
    captions: &[LayoutBoxId],
    width: f32,
    mut y: f32,
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
        set_structural_rect(
            world,
            row.id,
            origin.x,
            y,
            content_width,
            height,
            grid_size.width,
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
                grid_size.width,
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
            grid_size.width,
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
                grid_size.width,
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
    parent_width: f32,
) where
    N: Copy + Debug + Eq + Hash,
{
    let style = &world.boxes[id.index()].style.taffy;
    let padding = style
        .padding
        .resolve_or_zero(Some(parent_width), resolve_stylo_calc_value);
    let border = style
        .border
        .resolve_or_zero(Some(parent_width), resolve_stylo_calc_value);
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
        let cell = self.context.cells[cell_index].id;
        // The virtual table grid owns the used grid-item style: margins are
        // zero, column sizing has already consumed the applicable first-row
        // width, and cell height is a minimum contribution. Measuring the
        // subtree through the original box style would reintroduce widths from
        // later rows and make auto tracks overflow the table content box.
        let original = std::mem::replace(
            &mut self.world.boxes[cell.index()].style.taffy,
            self.context.cells[cell_index].style.clone(),
        );
        let output = self.world.compute_child_layout(cell.to_taffy(), inputs);
        self.world.boxes[cell.index()].style.taffy = original;
        output
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
