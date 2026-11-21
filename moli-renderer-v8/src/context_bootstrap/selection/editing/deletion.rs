use super::line_break::{
    create_break, enclosing_sibling_blocks, is_inline_container, merge_sibling_blocks, table,
    white_space,
};
use super::{
    RangeBoundarySide, collapse_editing_selection, editing_boundary_length, is_inline_formatting,
    is_text, lone_editor_break_with, new_editing_range, node_wrapper_from_handle, range,
    text_boundary,
};
use crate::context_bootstrap::selection::editing_caret::{
    adjacent_inline_content, is_collapsible_space, is_replaced_element,
};
use crate::context_bootstrap::selection::{selection_dispatch_change, selection_store};
use crate::context_bootstrap::selection_value_for_window;
use crate::document_runtime::DomHandle;
use crate::dom::forms::InputType;
use crate::dom::native::{DomHost, NodeType};
use crate::native_bridge::element::ComputedStyleRead;
use crate::native_bridge::{PageScriptHost, SelectionBoundarySnapshot};
use crate::range_boundary::point_order_in_dom;
use crate::style_engine::{ComputedDisplayKind, ComputedWhiteSpaceCollapseKind};
use icu_segmenter::GraphemeClusterSegmenter;

type Boundary = SelectionBoundarySnapshot;

enum AdjacentDeletion {
    Contents([Boundary; 2]),
    SelectTable([Boundary; 2]),
}

pub(crate) fn delete_editable_selection(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    forward: bool,
    is_editable: impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<bool> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let selection = runtime.document_editing_selection(document)?;
    let (start, end) = (selection.start, selection.end);
    if !is_editable(runtime, start.container) || !is_editable(runtime, end.container) {
        return None;
    }
    let order = point_order_in_dom(
        dom,
        start.container,
        start.offset,
        end.container,
        end.offset,
    )?;
    let [start, end] = if order.is_gt() {
        [end, start]
    } else {
        [start, end]
    };
    let backward = runtime.document_selection_is_backward(document);
    let anchor = if backward { end } else { start };
    if inert_ancestor(dom, anchor.container).is_some() {
        return Some(false);
    }

    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let bounds = if order.is_eq() {
        match adjacent_deletion(runtime, host, start, forward, &is_editable) {
            Some(AdjacentDeletion::Contents(bounds)) => bounds,
            Some(AdjacentDeletion::SelectTable([from, to])) => {
                let range = new_editing_range(scope, document, from, to)?;
                let anchor = node_wrapper_from_handle(scope, from.container)?;
                let focus = node_wrapper_from_handle(scope, to.container)?;
                selection_store(
                    scope,
                    selection,
                    range,
                    anchor,
                    from.offset,
                    focus,
                    to.offset,
                    "forward",
                );
                selection_dispatch_change(scope);
                return Some(false);
            }
            None => return Some(false),
        }
    } else {
        let focus = if backward { start } else { end };
        if let Some(inert) = inert_ancestor(dom, focus.container) {
            let edge = boundary_of_node(dom, inert, backward)?;
            if backward { [edge, end] } else { [start, edge] }
        } else {
            [start, end]
        }
    };
    let [from, to] = match table::selection_boundaries(runtime, bounds, &is_editable) {
        Some(bounds) => bounds,
        None if table::spans_partial_table_edges(dom, bounds) => bounds,
        None => return Some(false),
    };
    if from == to {
        return Some(false);
    }

    let blocks = enclosing_sibling_blocks(runtime, host, from.container, to.container);
    let removed_line_content = blocks.is_some()
        || (editing_block(runtime, host, from.container)
            == editing_block(runtime, host, to.container)
            && ([from, to]
                .into_iter()
                .any(|point| is_text(runtime, point.container))
                || (from.container == to.container
                    && adjacent_child(dom, from, true)
                        .is_some_and(|child| is_text(runtime, child)))));
    // The collapsed caret in a selected inline span carries its formatting
    // into subsequent typing, even when the deletion leaves its text empty.
    let retains_formatting = !order.is_eq()
        && from.container == to.container
        && is_text(runtime, from.container)
        && dom
            .parent_node(from.container)
            .is_some_and(|parent| parent != host && !is_rendered_block(runtime, parent));
    let caret = if order.is_eq() && forward && is_text(runtime, start.container) {
        start
    } else {
        from
    };
    let caret_range = new_editing_range(scope, document, caret, caret)?;
    let deletion_range = new_editing_range(scope, document, from, to)?;
    table::delete_contents(
        scope,
        runtime_ptr,
        document,
        deletion_range,
        [from, to],
        &is_editable,
    )?;

    // The caret's live range follows removals. Each cleanup rechecks native
    // ownership because removing a selected element can run author code.
    if !retains_formatting {
        for point in [from, to] {
            prune_empty_boundary(scope, runtime_ptr, host, point.container, &is_editable);
        }
    }
    if let Some((left, right, parent)) = blocks {
        join_blocks(scope, runtime_ptr, left, right, parent, &is_editable);
    }

    let caret = range::native_range_boundary_handles(scope, caret_range)?.start;
    let caret = Boundary {
        container: caret.container,
        offset: caret.offset,
    };
    let runtime = unsafe { &*runtime_ptr };
    if is_editable(runtime, caret.container) {
        if removed_line_content {
            pad_empty_line(scope, runtime_ptr, document, host, caret);
        }
        let runtime = unsafe { &*runtime_ptr };
        if is_editable(runtime, caret.container) {
            let caret = canonical_caret(runtime, host, caret);
            let _ = collapse_editing_selection(scope, selection, caret.container, caret.offset);
        }
    }
    Some(true)
}

fn adjacent_deletion(
    runtime: &PageScriptHost,
    host: DomHandle,
    caret: Boundary,
    forward: bool,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<AdjacentDeletion> {
    let dom = runtime.dom_host();
    let block = editing_block(runtime, host, caret.container);
    let mut point = caret;
    loop {
        let node = dom.node(point.container)?;
        if is_text(runtime, point.container) {
            // The visual edge of a different block is a separate deletion unit.
            if editing_block(runtime, host, point.container) != block {
                return None;
            }
            let parent = node.parent_node_id()?;
            let units = node.character_data_value()?.utf16_units();
            if let Some((from, to)) = adjacent_text_span(
                runtime,
                point.container,
                parent,
                &units,
                point.offset,
                forward,
            ) {
                return Some(AdjacentDeletion::Contents([
                    Boundary {
                        container: point.container,
                        offset: from,
                    },
                    Boundary {
                        container: point.container,
                        offset: to,
                    },
                ]));
            }
        } else if let Some(child) = adjacent_child(dom, point, forward) {
            let candidate = dom.node(child)?;
            let exit = boundary_of_node(dom, child, forward)?;
            if let Some(element) = candidate.as_element() {
                let style = ComputedStyleRead::new(runtime, child).rendered_style_facts()?;
                if style.display == ComputedDisplayKind::None
                    || style.hides_contents
                    || (element.is_html_input() && element.input_type() == InputType::Hidden)
                {
                    point = exit;
                    continue;
                }
                if element.is_html_element("table") {
                    // Forward Delete selects the table before a subsequent
                    // command can remove it. Backspace must leave the grid alone.
                    return forward.then_some(AdjacentDeletion::SelectTable([caret, exit]));
                }
                let line_break = element.is_html_element("br");
                let follows_block = forward
                    && adjacent_child(dom, point, false)
                        .is_some_and(|node| is_rendered_block(runtime, node));
                if line_break && !follows_block && terminal_break(runtime, host, child) {
                    point = exit;
                    continue;
                }
                let atom = || {
                    AdjacentDeletion::Contents(if forward {
                        [point, exit]
                    } else {
                        [exit, point]
                    })
                };
                if line_break
                    || element.is_html_element("hr")
                    || is_replaced_element(element)
                    || element.has_attribute("inert")
                    || !is_editable(runtime, child)
                    || (style.visibility_visible
                        && dom.first_child(child).is_none()
                        && element.local_name().contains('-'))
                {
                    return style.visibility_visible.then(atom);
                }
                if !is_inline_container(style.display) {
                    // A trailing blank line can be represented by a BR directly
                    // after a block, with the caret at their shared boundary.
                    if !forward
                        && point.container == block
                        && let Some(line_break) = adjacent_child(dom, point, true)
                        && dom.is_html_element_named(line_break, "br")
                        && std::iter::successors(dom.next_sibling(line_break), |&node| {
                            dom.next_sibling(node)
                        })
                        .find(|&node| !is_empty_or_comment(dom, node))
                        .is_none_or(|node| is_rendered_block(runtime, node))
                    {
                        return Some(AdjacentDeletion::Contents([
                            Boundary {
                                container: child,
                                offset: dom.child_handles(child).count() as u32,
                            },
                            boundary_of_node(dom, line_break, true)?,
                        ]));
                    }
                    if block == host
                        && (dom
                            .child_handles(child)
                            .all(|node| is_empty_or_comment(dom, node))
                            || lone_editor_break_with(dom, child, is_inline_formatting).is_some())
                    {
                        return Some(atom());
                    }
                    let (start, end) = if forward {
                        (block, child)
                    } else {
                        (child, block)
                    };
                    let (left, right, _) = enclosing_sibling_blocks(runtime, host, start, end)?;
                    if dom.next_sibling(left) != Some(right) {
                        return None;
                    }
                    return Some(AdjacentDeletion::Contents([
                        Boundary {
                            container: left,
                            offset: dom.child_handles(left).count() as u32,
                        },
                        Boundary {
                            container: right,
                            offset: 0,
                        },
                    ]));
                }
            }
            point = Boundary {
                container: child,
                offset: if forward {
                    0
                } else {
                    editing_boundary_length(dom, candidate, child) as u32
                },
            };
            continue;
        }

        if point.container == host
            || dom.is_html_element_named(point.container, "td")
            || dom.is_html_element_named(point.container, "th")
        {
            return None;
        }
        point = boundary_of_node(dom, point.container, forward)?;
    }
}

fn adjacent_child(dom: &DomHost, point: Boundary, forward: bool) -> Option<DomHandle> {
    let index = if forward {
        point.offset
    } else {
        point.offset.checked_sub(1)?
    };
    dom.child_handles(point.container).nth(index as usize)
}

fn boundary_of_node(dom: &DomHost, node: DomHandle, after: bool) -> Option<Boundary> {
    let container = dom.parent_node(node)?;
    let offset = dom.child_index(container, node)? as u32 + u32::from(after);
    Some(Boundary { container, offset })
}

fn inert_ancestor(dom: &DomHost, node: DomHandle) -> Option<DomHandle> {
    std::iter::successors(Some(node), |&node| dom.parent_node(node)).find(|&node| {
        dom.node(node)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.has_attribute("inert"))
    })
}

fn editing_block(runtime: &PageScriptHost, host: DomHandle, node: DomHandle) -> DomHandle {
    std::iter::successors(Some(node), |&node| runtime.dom_host().parent_node(node))
        .take_while(|&node| node != host)
        .find(|&node| is_rendered_block(runtime, node))
        .unwrap_or(host)
}

fn is_rendered_block(runtime: &PageScriptHost, node: DomHandle) -> bool {
    ComputedStyleRead::new(runtime, node)
        .rendered_style_facts()
        .is_some_and(|style| {
            style.display != ComputedDisplayKind::None && !is_inline_container(style.display)
        })
}

fn is_empty_or_comment(dom: &DomHost, node: DomHandle) -> bool {
    dom.node(node).is_some_and(|node| {
        node.node_type() == NodeType::Comment
            || node
                .character_data_value()
                .is_some_and(|data| data.utf16_units().is_empty())
    })
}

fn terminal_break(runtime: &PageScriptHost, host: DomHandle, line_break: DomHandle) -> bool {
    let dom = runtime.dom_host();
    let Some(parent) = dom.parent_node(line_break) else {
        return false;
    };
    let block = editing_block(runtime, host, parent);
    std::iter::successors(Some(line_break), |&node| dom.parent_node(node))
        .take_while(|&node| node != block)
        .all(|node| {
            std::iter::successors(dom.next_sibling(node), |&node| dom.next_sibling(node))
                .all(|sibling| is_empty_or_comment(dom, sibling))
        })
}

fn trailing_break(dom: &DomHost, mut node: DomHandle) -> Option<DomHandle> {
    loop {
        if dom.is_html_element_named(node, "br") {
            return Some(node);
        }
        let child = dom
            .child_handles(node)
            .filter(|&child| !is_empty_or_comment(dom, child))
            .last()?;
        if !dom.is_html_element_named(child, "br")
            && !dom.node(child).is_some_and(is_inline_formatting)
        {
            return None;
        }
        node = child;
    }
}

fn adjacent_text_span(
    runtime: &PageScriptHost,
    node: DomHandle,
    parent: DomHandle,
    units: &[u16],
    offset: u32,
    forward: bool,
) -> Option<(u32, u32)> {
    let offset = offset as usize;
    let at = if forward {
        offset
    } else {
        offset.checked_sub(1)?
    };
    let collapse = white_space(runtime, parent);
    let collapses = |unit| match collapse {
        ComputedWhiteSpaceCollapseKind::Collapse | ComputedWhiteSpaceCollapseKind::Other => {
            is_collapsible_space(unit)
        }
        ComputedWhiteSpaceCollapseKind::PreserveBreaks => matches!(unit, 0x09 | 0x0c | 0x20),
        ComputedWhiteSpaceCollapseKind::Preserve | ComputedWhiteSpaceCollapseKind::BreakSpaces => {
            false
        }
    };
    let (from, to) = if collapses(*units.get(at)?) {
        let from = at
            - units[..at]
                .iter()
                .rev()
                .take_while(|&&unit| collapses(unit))
                .count();
        let to = at
            + units[at..]
                .iter()
                .take_while(|&&unit| collapses(unit))
                .count();
        let before = from != 0 || adjacent_inline_content(runtime, node, false);
        let after = to != units.len() || adjacent_inline_content(runtime, node, true);
        match (before, after, forward) {
            (true, true, _) => (from, to),
            (true, false, false) if from > 0 => (grapheme_span(units, from - 1)?.0, to),
            (false, true, true) if to < units.len() => (from, grapheme_span(units, to)?.1),
            _ => return None,
        }
    } else {
        grapheme_span(units, at)?
    };
    Some((from as u32, to as u32))
}

fn grapheme_span(units: &[u16], at: usize) -> Option<(usize, usize)> {
    let mut from = 0;
    for to in GraphemeClusterSegmenter::new().segment_utf16(units).skip(1) {
        if at < to {
            return Some((from, to));
        }
        from = to;
    }
    None
}

fn prune_empty_boundary(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    host: DomHandle,
    mut node: DomHandle,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) {
    while node != host {
        let runtime = unsafe { &mut *runtime_ptr };
        let dom = runtime.dom_host();
        let Some(parent) = dom.parent_node(node) else {
            return;
        };
        let empty = dom.node(node).is_some_and(|data| {
            if is_text(runtime, node) {
                data.character_data_value()
                    .is_some_and(|data| data.utf16_units().is_empty())
            } else {
                is_inline_formatting(data) && dom.first_child(node).is_none()
            }
        });
        if !empty
            || !is_editable(runtime, node)
            || !is_editable(runtime, parent)
            || !runtime.remove_child(scope, runtime_ptr, parent, node)
        {
            return;
        }
        node = parent;
    }
}

fn join_blocks(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    left: DomHandle,
    right: DomHandle,
    parent: DomHandle,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    if dom.parent_node(left) != Some(parent)
        || dom.next_sibling(left) != Some(right)
        || !is_editable(runtime, left)
        || !is_editable(runtime, right)
    {
        return;
    }
    // Retain the left placeholder when both blocks are otherwise empty.
    let (block, redundant_break) =
        if let Some(line_break) = lone_editor_break_with(dom, right, is_inline_formatting) {
            (right, Some(line_break))
        } else {
            (left, trailing_break(dom, left))
        };
    if let Some(line_break) = redundant_break
        && let Some(container) = dom.parent_node(line_break)
        && is_editable(runtime, container)
    {
        let _ =
            unsafe { &mut *runtime_ptr }.remove_child(scope, runtime_ptr, container, line_break);
        prune_empty_boundary(scope, runtime_ptr, block, container, is_editable);
    }
    let _ = merge_sibling_blocks(scope, runtime_ptr, left, right, parent, is_editable);
}

fn canonical_caret(runtime: &PageScriptHost, host: DomHandle, caret: Boundary) -> Boundary {
    let dom = runtime.dom_host();
    let caret = adjacent_child(dom, caret, false)
        .filter(|&node| {
            dom.is_html_element_named(node, "br")
                || dom.node(node).is_some_and(is_inline_formatting)
        })
        .and_then(|node| trailing_break(dom, node))
        .filter(|&line_break| terminal_break(runtime, host, line_break))
        .and_then(|line_break| boundary_of_node(dom, line_break, false))
        .unwrap_or(caret);
    let text = if is_text(runtime, caret.container) {
        Some((caret.container, caret.offset))
    } else {
        text_boundary(dom, caret.container, caret.offset, RangeBoundarySide::End)
            .or_else(|| text_boundary(dom, caret.container, caret.offset, RangeBoundarySide::Start))
    };
    text.map_or(caret, |(container, offset)| Boundary { container, offset })
}

fn pad_empty_line(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    caret: Boundary,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    let dom = runtime.dom_host();
    if is_text(runtime, caret.container)
        || !is_rendered_block(runtime, editing_block(runtime, host, caret.container))
    {
        return;
    }
    let next = adjacent_child(dom, caret, true);
    let is_block = |node| is_rendered_block(runtime, node);
    if !next.is_none_or(is_block) || !adjacent_child(dom, caret, false).is_none_or(is_block) {
        return;
    }
    let line_break = create_break(runtime, document);
    let _ = runtime.insert_before(scope, runtime_ptr, caret.container, line_break, next);
}
