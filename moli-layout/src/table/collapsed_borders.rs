// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Conflict resolution and joint geometry follow Blink's `TableBorders` and
// `TablePainter::PaintCollapsedBorders` contracts. Unlike Blitz's current
// first-cell approximation, every winning edge retains its own width, style,
// color, and source order. The state remains pass-local and source-free.

use std::{fmt::Debug, hash::Hash};

use taffy::{Rect, ResolveOrZero, style_helpers};

use super::{TableCell, TableColumn, TableRow};
use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutRect, LayoutWorld, PaintBorderStyle, PaintColor,
    PaintEdgeSizes, ResolvedLayoutStyle, style::resolve_stylo_calc_value,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhysicalSide {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug)]
struct WinningEdge {
    width: f32,
    color: PaintColor,
    style: PaintBorderStyle,
    box_order: usize,
}

impl WinningEdge {
    fn can_paint(self) -> bool {
        self.width.is_finite()
            && self.width > 0.0
            && !matches!(
                self.style,
                PaintBorderStyle::None | PaintBorderStyle::Hidden
            )
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum EdgeSlot {
    #[default]
    Empty,
    DoNotFill,
    Winner(WinningEdge),
}

impl EdgeSlot {
    fn winner(self) -> Option<WinningEdge> {
        match self {
            Self::Winner(edge) => Some(edge),
            Self::Empty | Self::DoNotFill => None,
        }
    }
}

/// One already joint-adjusted edge ready for backend-neutral projection.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CollapsedTableBorderSegment {
    pub(crate) rect: LayoutRect,
    pub(crate) color: PaintColor,
    pub(crate) style: PaintBorderStyle,
    pub(crate) horizontal: bool,
}

/// Pass-owned conflict grid for one `border-collapse: collapse` table.
#[derive(Clone, Debug)]
pub(crate) struct CollapsedTableBorders {
    row_count: usize,
    column_count: usize,
    horizontal: Vec<EdgeSlot>,
    vertical: Vec<EdgeSlot>,
    segments: Vec<CollapsedTableBorderSegment>,
}

impl CollapsedTableBorders {
    fn new(row_count: usize, column_count: usize) -> Self {
        Self {
            row_count,
            column_count,
            horizontal: vec![EdgeSlot::Empty; (row_count + 1).saturating_mul(column_count)],
            vertical: vec![EdgeSlot::Empty; row_count.saturating_mul(column_count + 1)],
            segments: Vec::new(),
        }
    }

    pub(crate) fn segments(&self) -> &[CollapsedTableBorderSegment] {
        &self.segments
    }

    fn horizontal_index(&self, row: usize, column: usize) -> Option<usize> {
        (row <= self.row_count && column < self.column_count)
            .then_some(row.saturating_mul(self.column_count).saturating_add(column))
    }

    fn vertical_index(&self, row: usize, column: usize) -> Option<usize> {
        (row < self.row_count && column <= self.column_count).then_some(
            row.saturating_mul(self.column_count + 1)
                .saturating_add(column),
        )
    }

    fn horizontal_edge(&self, row: isize, column: isize) -> Option<WinningEdge> {
        let (Ok(row), Ok(column)) = (usize::try_from(row), usize::try_from(column)) else {
            return None;
        };
        self.horizontal_index(row, column)
            .and_then(|index| self.horizontal[index].winner())
    }

    fn vertical_edge(&self, row: isize, column: isize) -> Option<WinningEdge> {
        let (Ok(row), Ok(column)) = (usize::try_from(row), usize::try_from(column)) else {
            return None;
        };
        self.vertical_index(row, column)
            .and_then(|index| self.vertical[index].winner())
    }

    fn merge_box(
        &mut self,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        style: &ResolvedLayoutStyle,
        box_order: usize,
    ) {
        if row_span == 0 || column_span == 0 || self.row_count == 0 || self.column_count == 0 {
            return;
        }
        let row_end = row.saturating_add(row_span).min(self.row_count);
        let column_end = column.saturating_add(column_span).min(self.column_count);
        if row >= row_end || column >= column_end {
            return;
        }

        if let Some(edge) = border_edge(style, PhysicalSide::Top, box_order) {
            for current in column..column_end {
                if let Some(index) = self.horizontal_index(row, current) {
                    merge_edge(&mut self.horizontal[index], edge);
                }
            }
        }
        if let Some(edge) = border_edge(style, PhysicalSide::Bottom, box_order) {
            for current in column..column_end {
                if let Some(index) = self.horizontal_index(row_end, current) {
                    merge_edge(&mut self.horizontal[index], edge);
                }
            }
        }
        if let Some(edge) = border_edge(style, PhysicalSide::Left, box_order) {
            for current in row..row_end {
                if let Some(index) = self.vertical_index(current, column) {
                    merge_edge(&mut self.vertical[index], edge);
                }
            }
        }
        if let Some(edge) = border_edge(style, PhysicalSide::Right, box_order) {
            for current in row..row_end {
                if let Some(index) = self.vertical_index(current, column_end) {
                    merge_edge(&mut self.vertical[index], edge);
                }
            }
        }
    }

    fn mark_spanned_cell_interior(&mut self, cell: &TableCell) {
        let row_end = cell.row.saturating_add(cell.row_span).min(self.row_count);
        let column_end = cell
            .column
            .saturating_add(cell.column_span)
            .min(self.column_count);
        for row in cell.row..row_end {
            for column in cell.column.saturating_add(1)..column_end {
                if let Some(index) = self.vertical_index(row, column)
                    && matches!(self.vertical[index], EdgeSlot::Empty)
                {
                    self.vertical[index] = EdgeSlot::DoNotFill;
                }
            }
        }
        for row in cell.row.saturating_add(1)..row_end {
            for column in cell.column..column_end {
                if let Some(index) = self.horizontal_index(row, column)
                    && matches!(self.horizontal[index], EdgeSlot::Empty)
                {
                    self.horizontal[index] = EdgeSlot::DoNotFill;
                }
            }
        }
    }

    fn cell_strut(&self, cell: &TableCell) -> PaintEdgeSizes {
        self.range_strut(cell.row, cell.column, cell.row_span, cell.column_span)
    }

    fn range_strut(
        &self,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
    ) -> PaintEdgeSizes {
        let row_end = row.saturating_add(row_span).min(self.row_count);
        let column_end = column.saturating_add(column_span).min(self.column_count);
        let mut widths = PaintEdgeSizes::default();
        for current_row in row..row_end {
            widths.left = widths.left.max(paintable_width(
                self.vertical_index(current_row, column)
                    .and_then(|index| self.vertical[index].winner()),
            ));
            widths.right = widths.right.max(paintable_width(
                self.vertical_index(current_row, column_end)
                    .and_then(|index| self.vertical[index].winner()),
            ));
        }
        for current_column in column..column_end {
            widths.top = widths.top.max(paintable_width(
                self.horizontal_index(row, current_column)
                    .and_then(|index| self.horizontal[index].winner()),
            ));
            widths.bottom = widths.bottom.max(paintable_width(
                self.horizontal_index(row_end, current_column)
                    .and_then(|index| self.horizontal[index].winner()),
            ));
        }
        half(widths)
    }

    fn table_strut(&self) -> PaintEdgeSizes {
        if self.row_count == 0 || self.column_count == 0 {
            return PaintEdgeSizes::default();
        }
        self.range_strut(0, 0, self.row_count, self.column_count)
    }

    pub(crate) fn set_geometry(&mut self, columns: &[f32], rows: &[f32]) {
        self.segments.clear();
        if columns.len() != self.column_count + 1 || rows.len() != self.row_count + 1 {
            return;
        }

        // Blink stores vertical then horizontal edges at each grid
        // intersection. Keep that order so equal-precedence transparent joints
        // compose identically.
        for row in 0..=self.row_count {
            for column in 0..=self.column_count {
                if row < self.row_count
                    && let Some(edge) = self.vertical_edge(row as isize, column as isize)
                    && edge.can_paint()
                {
                    let (start_width, start_wins) = self.vertical_joint(row, column, true);
                    let (end_width, end_wins) = self.vertical_joint(row, column, false);
                    let mut start = rows[row];
                    let mut end = rows[row + 1];
                    if start_wins {
                        start -= start_width / 2.0;
                    } else {
                        start += start_width / 2.0;
                    }
                    if end_wins {
                        end += end_width / 2.0;
                    } else {
                        end -= end_width / 2.0;
                    }
                    if end > start {
                        self.segments.push(CollapsedTableBorderSegment {
                            rect: LayoutRect::new(
                                columns[column] - edge.width / 2.0,
                                start,
                                edge.width,
                                end - start,
                            ),
                            color: edge.color,
                            style: edge.style,
                            horizontal: false,
                        });
                    }
                }
                if column < self.column_count
                    && let Some(edge) = self.horizontal_edge(row as isize, column as isize)
                    && edge.can_paint()
                {
                    let (start_width, start_wins) = self.horizontal_joint(row, column, true);
                    let (end_width, end_wins) = self.horizontal_joint(row, column, false);
                    let mut start = columns[column];
                    let mut end = columns[column + 1];
                    if start_wins {
                        start -= start_width / 2.0;
                    } else {
                        start += start_width / 2.0;
                    }
                    if end_wins {
                        end += end_width / 2.0;
                    } else {
                        end -= end_width / 2.0;
                    }
                    if end > start {
                        self.segments.push(CollapsedTableBorderSegment {
                            rect: LayoutRect::new(
                                start,
                                rows[row] - edge.width / 2.0,
                                end - start,
                                edge.width,
                            ),
                            color: edge.color,
                            style: edge.style,
                            horizontal: true,
                        });
                    }
                }
            }
        }
    }

    fn horizontal_joint(&self, row: usize, column: usize, start: bool) -> (f32, bool) {
        let row = row as isize;
        let intersection = if start { column } else { column + 1 } as isize;
        let before = self.horizontal_edge(row, intersection - 1);
        let after = self.horizontal_edge(row, intersection);
        let over = self.vertical_edge(row - 1, intersection);
        let under = self.vertical_edge(row, intersection);
        let inline_compare = compare_for_paint(before, after);
        let block_compare = compare_for_paint(over, under);
        let inline = if inline_compare == 1 { before } else { after };
        let block = if block_compare == 1 { over } else { under };
        let cross = compare_for_paint(inline, block);
        let current_wins = if start {
            cross != -1 && inline_compare != 1
        } else {
            cross != -1 && inline_compare != -1
        };
        (block.map_or(0.0, |edge| edge.width), current_wins)
    }

    fn vertical_joint(&self, row: usize, column: usize, start: bool) -> (f32, bool) {
        let intersection_row = if start { row } else { row + 1 } as isize;
        let column = column as isize;
        let before = self.horizontal_edge(intersection_row, column - 1);
        let after = self.horizontal_edge(intersection_row, column);
        let over = self.vertical_edge(intersection_row - 1, column);
        let under = self.vertical_edge(intersection_row, column);
        let inline_compare = compare_for_paint(before, after);
        let block_compare = compare_for_paint(over, under);
        let inline = if inline_compare == 1 { before } else { after };
        let block = if block_compare == 1 { over } else { under };
        let cross = compare_for_paint(inline, block);
        let current_wins = if start {
            cross != 1 && block_compare != 1
        } else {
            cross != 1 && block_compare != -1
        };
        (inline.map_or(0.0, |edge| edge.width), current_wins)
    }
}

pub(super) fn prepare_collapsed_table_borders<N>(world: &mut LayoutWorld<N>, root: LayoutBoxId)
where
    N: Copy + Debug + Eq + Hash,
{
    let context = super::build_table_context(world, root);
    if !context.collapsed_borders {
        return;
    }

    let mut borders = CollapsedTableBorders::new(context.rows.len(), context.column_count);
    let mut box_order = 0usize;

    // CSS Tables conflict precedence is established by merge order. Equal
    // width/style candidates retain the first source: cell, row, row group,
    // column, column group, then table.
    for cell in &context.cells {
        box_order += 1;
        borders.merge_box(
            cell.row,
            cell.column,
            cell.row_span,
            cell.column_span,
            &world.boxes[cell.id.index()].style,
            box_order,
        );
        if cell.row_span > 1 || cell.column_span > 1 {
            borders.mark_spanned_cell_interior(cell);
        }
    }
    for row in &context.rows {
        box_order += 1;
        borders.merge_box(
            row.index,
            0,
            1,
            context.column_count,
            &world.boxes[row.id.index()].style,
            box_order,
        );
    }
    for (group, start, span) in row_groups(&context.rows) {
        box_order += 1;
        borders.merge_box(
            start,
            0,
            span,
            context.column_count,
            &world.boxes[group.index()].style,
            box_order,
        );
    }
    box_order += 1;
    let column_box_order = box_order;
    for column in context
        .columns
        .iter()
        .filter(|column| world.boxes[column.id.index()].kind == LayoutBoxKind::TableColumn)
    {
        for offset in 0..column.span {
            borders.merge_box(
                0,
                column.start + offset,
                context.rows.len(),
                1,
                &world.boxes[column.id.index()].style,
                column_box_order,
            );
        }
    }
    box_order += 1;
    let column_group_box_order = box_order;
    for (group, start, span) in column_groups(world, &context.columns) {
        borders.merge_box(
            0,
            start,
            context.rows.len(),
            span,
            &world.boxes[group.index()].style,
            column_group_box_order,
        );
    }
    box_order += 1;
    borders.merge_box(
        0,
        0,
        context.rows.len(),
        context.column_count,
        &world.boxes[root.index()].style,
        box_order,
    );

    let table_strut = borders.table_strut();
    set_layout_border(world, root, table_strut);
    world.boxes[root.index()].collapsed_table_border_part = true;
    for cell in &context.cells {
        let strut = borders.cell_strut(cell);
        set_layout_border(world, cell.id, strut);
        world.boxes[cell.id.index()].collapsed_table_border_part = true;
    }
    for part in context
        .rows
        .iter()
        .map(|row| row.id)
        .chain(context.rows.iter().filter_map(|row| row.group))
        .chain(context.columns.iter().map(|column| column.id))
        .chain(context.columns.iter().filter_map(|column| column.group))
    {
        set_layout_border(world, part, PaintEdgeSizes::default());
        world.boxes[part.index()].collapsed_table_border_part = true;
    }
    world.boxes[root.index()].collapsed_table_borders = Some(borders);
}

pub(super) fn set_collapsed_border_geometry<N>(
    world: &mut LayoutWorld<N>,
    root: LayoutBoxId,
    columns: &[f32],
    rows: &[f32],
) where
    N: Copy + Debug + Eq + Hash,
{
    if let Some(borders) = world.boxes[root.index()].collapsed_table_borders.as_mut() {
        borders.set_geometry(columns, rows);
    }
}

fn set_layout_border<N>(world: &mut LayoutWorld<N>, id: LayoutBoxId, widths: PaintEdgeSizes)
where
    N: Copy + Debug + Eq + Hash,
{
    world.boxes[id.index()].style.taffy.border = Rect {
        top: style_helpers::length(widths.top),
        right: style_helpers::length(widths.right),
        bottom: style_helpers::length(widths.bottom),
        left: style_helpers::length(widths.left),
    };
}

fn border_edge(
    style: &ResolvedLayoutStyle,
    side: PhysicalSide,
    box_order: usize,
) -> Option<WinningEdge> {
    let widths = style
        .taffy
        .border
        .resolve_or_zero(None, resolve_stylo_calc_value);
    let colors = style.border_colors();
    let styles = style.border_styles();
    let (width, color, border_style) = match side {
        PhysicalSide::Top => (widths.top, colors.top, styles.top),
        PhysicalSide::Right => (widths.right, colors.right, styles.right),
        PhysicalSide::Bottom => (widths.bottom, colors.bottom, styles.bottom),
        PhysicalSide::Left => (widths.left, colors.left, styles.left),
    };
    let style = collapsed_style(border_style);
    (style != PaintBorderStyle::None).then_some(WinningEdge {
        width: if width.is_finite() {
            width.max(0.0)
        } else {
            0.0
        },
        color,
        style,
        box_order,
    })
}

fn collapsed_style(style: PaintBorderStyle) -> PaintBorderStyle {
    match style {
        PaintBorderStyle::Inset => PaintBorderStyle::Ridge,
        PaintBorderStyle::Outset => PaintBorderStyle::Groove,
        other => other,
    }
}

fn merge_edge(slot: &mut EdgeSlot, source: WinningEdge) {
    if matches!(slot, EdgeSlot::DoNotFill) {
        return;
    }
    let EdgeSlot::Winner(current) = *slot else {
        *slot = EdgeSlot::Winner(source);
        return;
    };
    if source.style == PaintBorderStyle::Hidden {
        *slot = EdgeSlot::Winner(source);
        return;
    }
    if current.style == PaintBorderStyle::Hidden {
        return;
    }
    if source.width > current.width
        || (source.width == current.width
            && border_style_priority(source.style) > border_style_priority(current.style))
    {
        *slot = EdgeSlot::Winner(source);
    }
}

fn compare_for_paint(lhs: Option<WinningEdge>, rhs: Option<WinningEdge>) -> i8 {
    let lhs_paints = lhs.is_some_and(WinningEdge::can_paint);
    let rhs_paints = rhs.is_some_and(WinningEdge::can_paint);
    match (lhs_paints, rhs_paints) {
        (true, false) => return 1,
        (false, true) => return -1,
        (false, false) => return 0,
        (true, true) => {}
    }
    let (Some(lhs), Some(rhs)) = (lhs, rhs) else {
        return 0;
    };
    if lhs.width != rhs.width {
        return if lhs.width > rhs.width { 1 } else { -1 };
    }
    if lhs.style != rhs.style {
        return if border_style_priority(lhs.style) > border_style_priority(rhs.style) {
            1
        } else {
            -1
        };
    }
    match lhs.box_order.cmp(&rhs.box_order) {
        std::cmp::Ordering::Less => 1,
        std::cmp::Ordering::Greater => -1,
        std::cmp::Ordering::Equal => 0,
    }
}

fn border_style_priority(style: PaintBorderStyle) -> u8 {
    match collapsed_style(style) {
        PaintBorderStyle::None => 0,
        PaintBorderStyle::Groove | PaintBorderStyle::Outset => 1,
        PaintBorderStyle::Ridge | PaintBorderStyle::Inset => 3,
        PaintBorderStyle::Dotted => 4,
        PaintBorderStyle::Dashed => 5,
        PaintBorderStyle::Solid => 6,
        PaintBorderStyle::Double => 7,
        PaintBorderStyle::Hidden => 8,
    }
}

fn paintable_width(edge: Option<WinningEdge>) -> f32 {
    edge.filter(|edge| edge.can_paint())
        .map_or(0.0, |edge| edge.width)
}

fn half(widths: PaintEdgeSizes) -> PaintEdgeSizes {
    PaintEdgeSizes::new(
        widths.top / 2.0,
        widths.right / 2.0,
        widths.bottom / 2.0,
        widths.left / 2.0,
    )
}

fn row_groups(rows: &[TableRow]) -> Vec<(LayoutBoxId, usize, usize)> {
    let mut groups = Vec::new();
    for row in rows {
        let Some(group) = row.group else {
            continue;
        };
        if groups.last().is_some_and(|(last, _, _)| *last == group) {
            if let Some((_, _, span)) = groups.last_mut() {
                *span += 1;
            }
        } else {
            groups.push((group, row.index, 1));
        }
    }
    groups
}

fn column_groups<N>(
    world: &LayoutWorld<N>,
    columns: &[TableColumn],
) -> Vec<(LayoutBoxId, usize, usize)>
where
    N: Copy + Debug + Eq + Hash,
{
    let mut groups = Vec::<(LayoutBoxId, usize, usize)>::new();
    for column in columns {
        let group = if world.boxes[column.id.index()].kind == LayoutBoxKind::TableColumnGroup {
            Some(column.id)
        } else {
            column.group
        };
        let Some(group) = group else {
            continue;
        };
        if let Some(existing) = groups.iter_mut().find(|(id, _, _)| *id == group) {
            existing.1 = existing.1.min(column.start);
            existing.2 = existing.2.max(
                column
                    .start
                    .saturating_add(column.span)
                    .saturating_sub(existing.1),
            );
        } else {
            groups.push((group, column.start, column.span));
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: PaintColor = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    const BLUE: PaintColor = PaintColor::new(0.0, 0.0, 1.0, 1.0);

    fn edge(width: f32, style: PaintBorderStyle, color: PaintColor, order: usize) -> WinningEdge {
        WinningEdge {
            width,
            color,
            style,
            box_order: order,
        }
    }

    #[test]
    fn conflict_resolution_uses_hidden_width_style_then_first_source() {
        let mut slot = EdgeSlot::Empty;
        merge_edge(&mut slot, edge(4.0, PaintBorderStyle::Solid, RED, 1));
        merge_edge(&mut slot, edge(6.0, PaintBorderStyle::Dotted, BLUE, 2));
        assert_eq!(slot.winner().unwrap().color, BLUE, "wider edge wins");

        merge_edge(&mut slot, edge(6.0, PaintBorderStyle::Solid, RED, 3));
        assert_eq!(slot.winner().unwrap().color, RED, "stronger style wins");

        merge_edge(&mut slot, edge(6.0, PaintBorderStyle::Solid, BLUE, 4));
        assert_eq!(
            slot.winner().unwrap().color,
            RED,
            "equal candidates retain the earlier source"
        );

        merge_edge(&mut slot, edge(0.0, PaintBorderStyle::Hidden, BLUE, 5));
        assert_eq!(slot.winner().unwrap().style, PaintBorderStyle::Hidden);
        assert!(!slot.winner().unwrap().can_paint());
    }

    #[test]
    fn joint_geometry_extends_the_wider_winning_edge_without_gaps() {
        let mut borders = CollapsedTableBorders::new(1, 1);
        borders.horizontal[0] = EdgeSlot::Winner(edge(4.0, PaintBorderStyle::Solid, RED, 1));
        borders.vertical[0] = EdgeSlot::Winner(edge(8.0, PaintBorderStyle::Solid, BLUE, 2));
        borders.set_geometry(&[10.0, 50.0], &[10.0, 30.0]);

        let horizontal = borders
            .segments()
            .iter()
            .find(|segment| segment.horizontal)
            .expect("horizontal edge");
        let vertical = borders
            .segments()
            .iter()
            .find(|segment| !segment.horizontal)
            .expect("vertical edge");
        assert_eq!(horizontal.rect, LayoutRect::new(14.0, 8.0, 36.0, 4.0));
        assert_eq!(vertical.rect, LayoutRect::new(6.0, 8.0, 8.0, 22.0));
    }
}
