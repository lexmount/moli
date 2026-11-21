use super::{DomHandle, PageScriptHost, XHTML_NS, create_break, is_text, new_editing_range, range};
use crate::context_bootstrap::range_algorithms::range_delete_contents;
use crate::dom::native::DomHost;
use crate::native_bridge::SelectionBoundarySnapshot;
use crate::range_boundary::point_order_in_dom;

pub(in crate::context_bootstrap::selection::editing) fn selection_boundaries(
    runtime: &PageScriptHost,
    [start, end]: [SelectionBoundarySnapshot; 2],
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<[SelectionBoundarySnapshot; 2]> {
    let dom = runtime.dom_host();
    let table = enclosing_table(dom, start.container);
    // The cell-local edit cannot safely delete a selection that cuts across
    // the edge of a table. A table selected from outside uses the normal range.
    if table != enclosing_table(dom, end.container) {
        return None;
    }
    let Some(table) = table else {
        return Some([start, end]);
    };
    let in_grid = |point: SelectionBoundarySnapshot| {
        dom.node(point.container).is_some_and(|node| {
            node.namespace() == Some(XHTML_NS)
                && matches!(
                    node.local_name(),
                    Some("table" | "thead" | "tbody" | "tfoot" | "tr")
                )
        })
    };
    let compare = |a: SelectionBoundarySnapshot, b: SelectionBoundarySnapshot| {
        point_order_in_dom(dom, a.container, a.offset, b.container, b.offset)
    };
    let cells = || cells_in_table(dom, table).filter(|&cell| is_editable(runtime, cell));
    // Element boundary points can describe a row or a set of cells. Move
    // those endpoints into the cells before creating the deletion range.
    let start = if in_grid(start) {
        cells()
            .map(|container| SelectionBoundarySnapshot {
                container,
                offset: 0,
            })
            .find(|&point| compare(start, point).is_some_and(|order| order.is_le()))?
    } else {
        start
    };
    let end = if in_grid(end) {
        cells()
            .rev()
            .map(|container| SelectionBoundarySnapshot {
                container,
                offset: dom.child_handles(container).count() as u32,
            })
            .find(|&point| compare(point, end).is_some_and(|order| order.is_le()))?
    } else {
        end
    };
    (enclosing_cell(dom, start.container).is_some() == enclosing_cell(dom, end.container).is_some()
        && compare(start, end)?.is_le())
    .then_some([start, end])
}

// A range entering the first cell or leaving the last one can delete
// contents on both sides of the table edge without removing grid nodes.
pub(in crate::context_bootstrap::selection::editing) fn spans_partial_table_edges(
    dom: &DomHost,
    [start, end]: [SelectionBoundarySnapshot; 2],
) -> bool {
    let first = enclosing_table(dom, start.container);
    let last = enclosing_table(dom, end.container);
    let at_edge = |point: SelectionBoundarySnapshot, table, trailing| {
        let Some(parent) = dom.parent_node(table) else {
            return false;
        };
        if enclosing_table(dom, parent).is_some() {
            return false;
        }
        let mut cells = cells_in_table(dom, table);
        let edge_cell = if trailing {
            cells.next_back()
        } else {
            cells.next()
        };
        edge_cell.is_some() && enclosing_cell(dom, point.container) == edge_cell
    };
    first != last
        && first.is_none_or(|table| at_edge(start, table, true))
        && last.is_none_or(|table| at_edge(end, table, false))
}

pub(in crate::context_bootstrap::selection::editing) fn delete_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    range: v8::Local<'s, v8::Object>,
    [start, end]: [SelectionBoundarySnapshot; 2],
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<()> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let Some((first, last, table)) = spanning_cells(dom, start.container, end.container) else {
        return range_delete_contents(scope, range);
    };
    // Keep the table grid intact. Range.deleteContents on the original
    // endpoints would remove fully selected cells and rows.
    let cells = cells_in_table(dom, table).collect::<Vec<_>>();
    let from = cells.iter().position(|&cell| cell == first)?;
    let to = cells.iter().position(|&cell| cell == last)?;
    let segments = cells
        .get(from..=to)?
        .iter()
        .copied()
        .filter(|&cell| is_editable(runtime, cell))
        .map(|cell| {
            let from = if cell == first {
                start
            } else {
                SelectionBoundarySnapshot {
                    container: cell,
                    offset: 0,
                }
            };
            let to = if cell == last {
                end
            } else {
                SelectionBoundarySnapshot {
                    container: cell,
                    offset: dom.child_handles(cell).count() as u32,
                }
            };
            (cell, from, to)
        })
        .collect::<Vec<_>>();
    let live_ranges = segments
        .into_iter()
        .map(|(cell, from, to)| Some((cell, new_editing_range(scope, document, from, to)?)))
        .collect::<Option<Vec<_>>>()?;
    for (cell, range) in live_ranges {
        let bounds = range::native_range_boundary_handles(scope, range)?;
        let runtime = unsafe { &*runtime_ptr };
        let dom = runtime.dom_host();
        // Tree mutation reactions can move a cell or its contents.
        if enclosing_table(dom, cell) != Some(table)
            || [bounds.start.container, bounds.end.container]
                .into_iter()
                .any(|node| enclosing_cell(dom, node) != Some(cell) || !is_editable(runtime, node))
        {
            return None;
        }
        range_delete_contents(scope, range)?;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let dom = runtime.dom_host();
    let row = dom.parent_node(first);
    let empty = |cell| {
        dom.child_handles(cell).all(|child| {
            is_text(runtime, child)
                && dom
                    .node(child)
                    .and_then(|node| node.character_data_value())
                    .is_some_and(|data| data.utf16_units().is_empty())
        })
    };
    if row.is_some()
        && row == dom.parent_node(last)
        && enclosing_table(dom, last) == Some(table)
        && is_editable(runtime, last)
        && empty(first)
        && empty(last)
    {
        let placeholder = create_break(runtime, document);
        runtime
            .append_child(scope, runtime_ptr, last, placeholder)
            .then_some(())?;
    }
    Some(())
}

fn spanning_cells(
    dom: &DomHost,
    start: DomHandle,
    end: DomHandle,
) -> Option<(DomHandle, DomHandle, DomHandle)> {
    let first = enclosing_cell(dom, start)?;
    let last = enclosing_cell(dom, end)?;
    let table = enclosing_table(dom, first)?;
    (first != last && enclosing_table(dom, last) == Some(table)).then_some((first, last, table))
}

fn enclosing_cell(dom: &DomHost, node: DomHandle) -> Option<DomHandle> {
    std::iter::successors(Some(node), |&node| dom.parent_node(node))
        .find(|&node| is_cell(dom, node))
}

fn is_cell(dom: &DomHost, node: DomHandle) -> bool {
    dom.is_html_element_named(node, "td") || dom.is_html_element_named(node, "th")
}

fn enclosing_table(dom: &DomHost, node: DomHandle) -> Option<DomHandle> {
    std::iter::successors(Some(node), |&node| dom.parent_node(node))
        .find(|&node| dom.is_html_element_named(node, "table"))
}

fn cells_in_table(
    dom: &DomHost,
    table: DomHandle,
) -> impl DoubleEndedIterator<Item = DomHandle> + '_ {
    dom.elements_by_tag_name(table, "*", true)
        .into_iter()
        .filter(move |&node| is_cell(dom, node) && enclosing_table(dom, node) == Some(table))
}
