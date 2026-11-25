//! The CSS table grid's structure, shared across one numeric layout pass.

use std::{fmt::Debug, hash::Hash, ops::Range};

use crate::{LayoutBoxId, LayoutBoxKind, LayoutWorld};

#[derive(Clone, Copy, Debug)]
pub(super) struct TableCell {
    pub(super) id: LayoutBoxId,
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) row_span: usize,
    pub(super) column_span: usize,
}

#[derive(Debug)]
pub(super) struct TableRow {
    pub(super) id: LayoutBoxId,
    pub(super) group: Option<LayoutBoxId>,
    cells: Range<usize>,
}

#[derive(Debug)]
pub(super) struct TableColumn {
    pub(super) id: LayoutBoxId,
    pub(super) group: Option<LayoutBoxId>,
    pub(super) start: usize,
    pub(super) span: usize,
}

impl TableColumn {
    pub(super) fn tracks(&self) -> Range<usize> {
        self.start..self.start.saturating_add(self.span)
    }
}

#[derive(Debug)]
pub(super) struct TableSection {
    pub(super) id: LayoutBoxId,
    pub(super) rows: Range<usize>,
    pub(super) is_body: bool,
}

/// Box identities, visual section order, and resolved row and column spans.
/// Numeric consumers take current styles after collapsed border preparation.
#[derive(Debug)]
pub(crate) struct TableStructure {
    pub(super) cells: Vec<TableCell>,
    pub(super) rows: Vec<TableRow>,
    pub(super) columns: Vec<TableColumn>,
    pub(super) captions: Vec<LayoutBoxId>,
    pub(super) sections: Vec<TableSection>,
    pub(super) column_count: usize,
}

impl TableStructure {
    pub(super) fn first_row_cells(&self) -> &[TableCell] {
        self.rows
            .first()
            .map_or(&[], |row| &self.cells[row.cells.clone()])
    }

    pub(super) fn column_groups(&self) -> impl Iterator<Item = (LayoutBoxId, Range<usize>)> + '_ {
        // Each column group is collected as one contiguous run.
        self.columns
            .chunk_by(|left, right| left.group == right.group)
            .filter_map(|columns| {
                let first = columns.first()?;
                Some((first.group?, first.start..columns.last()?.tracks().end))
            })
    }

    pub(super) fn collect<N>(world: &LayoutWorld<N>, root: LayoutBoxId) -> Self
    where
        N: Copy + Debug + Eq + Hash,
    {
        let children = TableGroupedChildren::collect(world, root);
        let mut cells = Vec::new();
        let mut rows = Vec::new();
        let mut columns = Vec::new();
        let mut sections = Vec::new();
        let mut column_count = 0;
        for &column in &children.columns {
            collect_columns(world, column, None, &mut columns, &mut column_count);
        }
        for section in children.sections() {
            let start = rows.len();
            collect_rows(world, section, None, &mut rows, &mut cells);
            sections.push(TableSection {
                id: section,
                rows: start..rows.len(),
                is_body: Some(section) != children.header && Some(section) != children.footer,
            });
        }
        place_table_cells(&mut cells, &rows, &mut column_count);
        Self {
            cells,
            rows,
            columns,
            captions: children.captions,
            sections,
            column_count: column_count.max(1),
        }
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

enum ColumnCollectStep {
    Enter {
        current: LayoutBoxId,
        group: Option<LayoutBoxId>,
    },
    FinishGroup {
        current: LayoutBoxId,
        before: usize,
    },
}

fn collect_columns<N>(
    world: &LayoutWorld<N>,
    root: LayoutBoxId,
    root_group: Option<LayoutBoxId>,
    columns: &mut Vec<TableColumn>,
    column_count: &mut usize,
) where
    N: Copy + Debug + Eq + Hash,
{
    let mut stack = vec![ColumnCollectStep::Enter {
        current: root,
        group: root_group,
    }];
    while let Some(step) = stack.pop() {
        match step {
            ColumnCollectStep::Enter { current, group } => {
                match world.boxes[current.index()].kind {
                    LayoutBoxKind::TableColumnGroup => {
                        let before = *column_count;
                        stack.push(ColumnCollectStep::FinishGroup { current, before });
                        stack.extend(
                            world.boxes[current.index()]
                                .children
                                .iter()
                                .rev()
                                .copied()
                                .map(|child| ColumnCollectStep::Enter {
                                    current: child,
                                    group: Some(current),
                                }),
                        );
                    }
                    LayoutBoxKind::TableColumn => {
                        let span = table_data(world, current).span.max(1) as usize;
                        columns.push(TableColumn {
                            id: current,
                            group,
                            start: *column_count,
                            span,
                        });
                        *column_count += span;
                    }
                    _ => {}
                }
            }
            ColumnCollectStep::FinishGroup { current, before } => {
                if *column_count == before {
                    let span = table_data(world, current).span.max(1) as usize;
                    columns.push(TableColumn {
                        id: current,
                        group: Some(current),
                        start: *column_count,
                        span,
                    });
                    *column_count += span;
                }
            }
        }
    }
}

fn collect_rows<N>(
    world: &LayoutWorld<N>,
    root: LayoutBoxId,
    root_group: Option<LayoutBoxId>,
    rows: &mut Vec<TableRow>,
    cells: &mut Vec<TableCell>,
) where
    N: Copy + Debug + Eq + Hash,
{
    let mut stack = vec![(root, root_group)];
    while let Some((current, group)) = stack.pop() {
        match world.boxes[current.index()].kind {
            LayoutBoxKind::TableRowGroup
            | LayoutBoxKind::TableHeaderGroup
            | LayoutBoxKind::TableFooterGroup
            | LayoutBoxKind::AnonymousTableRowGroup => {
                stack.extend(
                    world.boxes[current.index()]
                        .children
                        .iter()
                        .rev()
                        .copied()
                        .map(|child| (child, Some(current))),
                );
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
                    cells.push(TableCell {
                        id: cell,
                        row: row_index,
                        column: 0,
                        row_span,
                        column_span,
                    });
                }
                rows.push(TableRow {
                    id: current,
                    group,
                    cells: start_cell_index..cells.len(),
                });
            }
            _ => {}
        }
    }
}

/// Consecutive ungrouped rows share a placement boundary too.
pub(super) fn row_groups(rows: &[TableRow]) -> impl Iterator<Item = (usize, &[TableRow])> {
    let mut next_row = 0;
    rows.chunk_by(|left, right| left.group == right.group)
        .map(move |group| {
            let start = next_row;
            next_row += group.len();
            (start, group)
        })
}

fn place_table_cells(cells: &mut [TableCell], rows: &[TableRow], max_columns: &mut usize) {
    let mut occupied_until = Vec::<usize>::new();

    for (start, group) in row_groups(rows) {
        occupied_until.clear();
        for (offset, row) in group.iter().enumerate() {
            let row_index = start + offset;
            let remaining_rows = group.len() - offset;
            let mut cursor = 0usize;
            for cell in &mut cells[row.cells.clone()] {
                let span = cell.column_span.max(1);
                loop {
                    let end = cursor.saturating_add(span);
                    if occupied_until.len() < end {
                        occupied_until.resize(end, 0);
                    }
                    if occupied_until[cursor..end]
                        .iter()
                        .all(|occupied| *occupied <= row_index)
                    {
                        cell.column = cursor;
                        cell.row_span = if cell.row_span == 0 {
                            remaining_rows
                        } else {
                            cell.row_span.min(remaining_rows)
                        };
                        occupied_until[cursor..end].fill(row_index + cell.row_span);
                        cursor = end;
                        *max_columns = (*max_columns).max(end);
                        break;
                    }
                    cursor += 1;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_group_ranges_include_span_only_groups() {
        let id = LayoutBoxId::from_index;
        let column = |index, group: Option<usize>, start, span| TableColumn {
            id: id(index),
            group: group.map(id),
            start,
            span,
        };
        let structure = TableStructure {
            cells: Vec::new(),
            rows: Vec::new(),
            columns: vec![
                column(1, None, 0, 1),
                column(2, Some(2), 1, 3),
                column(4, Some(3), 4, 2),
                column(5, Some(3), 6, 1),
                column(6, None, 7, 2),
            ],
            captions: Vec::new(),
            sections: Vec::new(),
            column_count: 9,
        };
        assert_eq!(
            structure.column_groups().collect::<Vec<_>>(),
            [(id(2), 1..4), (id(3), 4..7)]
        );
    }

    #[test]
    fn placement_clips_spans_at_group_boundaries_including_ungrouped_rows() {
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
                cells: cells.len()..cells.len() + cell_count,
            });
            for _ in 0..cell_count {
                let (column_span, row_span) = spans[cells.len()];
                cells.push(TableCell {
                    id: LayoutBoxId::from_index(30 + cells.len()),
                    row: index,
                    column: 0,
                    row_span,
                    column_span,
                });
            }
        }
        assert_eq!(
            row_groups(&rows)
                .map(|(start, group)| start..start + group.len())
                .collect::<Vec<_>>(),
            [0..3, 3..5, 5..6, 6..7]
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
}
