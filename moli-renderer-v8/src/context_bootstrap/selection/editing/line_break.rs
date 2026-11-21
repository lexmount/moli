use super::{
    DomHandle, NodeType, PageScriptHost, RangeBoundarySide, XHTML_NS, collapse_editing_selection,
    expand_rich_replacement_boundary, fragment_insertion_point, is_paragraph_or_heading, is_text,
    new_editing_range, node_wrapper_from_handle, range, range_insert_node_at_boundary,
    replace_text_span, selection_value_for_window,
};
use crate::context_bootstrap::selection::editing_caret::{
    is_collapsible_space, is_replaced_element,
};
use crate::dom::native::{ContentEditableState, Element};
use crate::native_bridge::element::ComputedStyleRead;
use crate::style_engine::{ComputedDisplayKind, ComputedWhiteSpaceCollapseKind};

pub(super) mod table;

pub(crate) fn replace_editable_selection_line_break(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    insertion_point_is_editable: impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<bool> {
    let insertion = prepare_break(
        scope,
        runtime_ptr,
        document,
        host,
        &insertion_point_is_editable,
    )?;
    insert_line_break_at_range(
        scope,
        runtime_ptr,
        document,
        host,
        insertion,
        &insertion_point_is_editable,
        false,
    )
}

pub(super) struct BreakInsertion<'s> {
    pub(super) range: v8::Local<'s, v8::Object>,
    pub(super) selection: v8::Local<'s, v8::Object>,
    pub(super) deleted_selection: bool,
}

pub(super) fn prepare_break<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    insertion_point_is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<BreakInsertion<'s>> {
    let runtime = unsafe { &*runtime_ptr };
    let snapshot = runtime.document_editing_selection(document)?;
    let (start, end) = (snapshot.start, snapshot.end);
    let order = crate::range_boundary::point_order_in_dom(
        runtime.dom_host(),
        start.container,
        start.offset,
        end.container,
        end.offset,
    )?;
    let (start, end) = if order.is_gt() {
        (end, start)
    } else {
        (start, end)
    };
    let [start, mut end] = if order.is_eq() {
        [start, end]
    } else {
        table::selection_boundaries(runtime, [start, end], insertion_point_is_editable)?
    };
    let blocks = if order.is_eq() {
        None
    } else {
        let blocks = enclosing_sibling_blocks(runtime, host, start.container, end.container);
        end = expand_rich_replacement_boundary(
            runtime.dom_host(),
            end,
            host,
            RangeBoundarySide::End,
            false,
        );
        blocks
    };
    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let range = new_editing_range(scope, document, start, end)?;
    // The deletion range can collapse outside the starting element when the
    // selection spans siblings. A live start point stays in the retained prefix.
    let insertion_range = if order.is_eq() {
        range
    } else {
        new_editing_range(scope, document, start, start)?
    };
    table::delete_contents(
        scope,
        runtime_ptr,
        document,
        range,
        [start, end],
        insertion_point_is_editable,
    )?;
    if let Some((left, right, parent)) = blocks {
        merge_sibling_blocks(
            scope,
            runtime_ptr,
            left,
            right,
            parent,
            insertion_point_is_editable,
        )?;
    }
    Some(BreakInsertion {
        range: insertion_range,
        selection,
        deleted_selection: !order.is_eq(),
    })
}

pub(super) fn insert_line_break_at_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    insertion: BreakInsertion<'s>,
    insertion_point_is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
    force_element_break: bool,
) -> Option<bool> {
    let BreakInsertion {
        range: insertion_range,
        selection,
        deleted_selection,
    } = insertion;
    let start = range::native_range_boundary_handles(scope, insertion_range)?.start;

    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let (container, mut offset) = fragment_insertion_point(runtime, start.container, start.offset);
    if !insertion_point_is_editable(runtime, container) {
        return None;
    }
    let parent = if is_text(runtime, container) {
        dom.parent_node(container)?
    } else {
        container
    };
    let element = dom.node(parent)?.as_element()?;
    if !allows_line_break(element) {
        return Some(deleted_selection);
    }
    if parent == container
        && let Some(previous) = offset
            .checked_sub(1)
            .and_then(|i| dom.child_handles(parent).nth(i as usize))
        && dom
            .node(previous)
            .is_some_and(|node| node.is_html_element_named("br"))
        && following_line_content(runtime, host, parent, offset).is_none()
    {
        // Reuse the existing final break as the caret placeholder.
        offset -= 1;
    }
    let preserves_newlines = !force_element_break
        && matches!(
            white_space(runtime, parent),
            ComputedWhiteSpaceCollapseKind::Preserve
                | ComputedWhiteSpaceCollapseKind::PreserveBreaks
                | ComputedWhiteSpaceCollapseKind::BreakSpaces
        );
    let runtime = unsafe { &mut *runtime_ptr };
    let inserted = if preserves_newlines {
        runtime.create_text_node_for_document(document, "\n")
    } else {
        create_break(runtime, document)
    };
    let container = node_wrapper_from_handle(scope, container)?;
    let line_break = node_wrapper_from_handle(scope, inserted)?;
    range_insert_node_at_boundary(scope, insertion_range, container, offset, line_break)?;

    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let parent = dom.parent_node(inserted)?;
    if !insertion_point_is_editable(runtime, parent) {
        return None;
    }
    let offset = dom.child_index(parent, inserted)? as u32 + 1;
    let (caret, offset) = match following_line_content(runtime, host, parent, offset) {
        Some(next) if is_text(runtime, next) && insertion_point_is_editable(runtime, next) => {
            let data = dom.node(next)?.character_data_value()?;
            let prefix = collapsed_prefix(runtime, dom.parent_node(next)?, &data.utf16_units());
            if prefix != 0 {
                // The leading space would disappear once the suffix starts a
                // new line. Preserve the collapsed space as one NBSP.
                replace_text_span(scope, runtime_ptr, next, 0..prefix as u32, &[0xa0])?;
            }
            (next, 0)
        }
        Some(_) => (parent, offset),
        None => {
            // A terminal break needs a following placeholder to make the
            // blank line reachable without adding a second textual newline.
            let padding = create_break(unsafe { &mut *runtime_ptr }, document);
            let container = node_wrapper_from_handle(scope, parent)?;
            let node = node_wrapper_from_handle(scope, padding)?;
            range_insert_node_at_boundary(scope, insertion_range, container, offset, node)?;
            (parent, offset)
        }
    };
    collapse_editing_selection(scope, selection, caret, offset)?;
    Some(true)
}

pub(super) fn enclosing_sibling_blocks(
    runtime: &PageScriptHost,
    host: DomHandle,
    start: DomHandle,
    end: DomHandle,
) -> Option<(DomHandle, DomHandle, DomHandle)> {
    let dom = runtime.dom_host();
    let block = |node| {
        std::iter::successors(Some(node), |&node| dom.parent_node(node))
            .take_while(|&node| node != host)
            .filter(|&node| dom.node(node).is_some_and(|node| node.is_element()))
            .find(|&node| {
                ComputedStyleRead::new(runtime, node)
                    .rendered_style_facts()
                    .is_some_and(|style| !is_inline_container(style.display))
            })
            .filter(|&node| {
                dom.node(node).is_some_and(|node| {
                    is_paragraph_or_heading(node)
                        || node.as_element().is_some_and(|element| {
                            element.namespace() == XHTML_NS
                                && matches!(element.local_name(), "div" | "li" | "dt" | "dd")
                        })
                })
            })
    };
    let left = block(start)?;
    let right = block(end)?;
    let parent = dom.parent_node(left)?;
    (left != right && dom.parent_node(right) == Some(parent)).then_some((left, right, parent))
}

pub(super) fn merge_sibling_blocks(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    left: DomHandle,
    right: DomHandle,
    parent: DomHandle,
    insertion_point_is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<()> {
    let adjacent = |runtime: &PageScriptHost| {
        let dom = runtime.dom_host();
        dom.parent_node(left) == Some(parent)
            && dom.next_sibling(left) == Some(right)
            && insertion_point_is_editable(runtime, left)
            && insertion_point_is_editable(runtime, right)
    };
    let runtime = unsafe { &*runtime_ptr };
    if !adjacent(runtime) {
        return Some(());
    }
    let tail = runtime.dom_host().child_handles(right).collect::<Vec<_>>();
    for child in tail {
        let runtime = unsafe { &mut *runtime_ptr };
        // A callback can move either block or an as-yet unmoved child.
        if !adjacent(runtime) || runtime.dom_host().parent_node(child) != Some(right) {
            return None;
        }
        runtime
            .append_child(scope, runtime_ptr, left, child)
            .then_some(())?;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    if adjacent(runtime) && runtime.dom_host().first_child(right).is_none() {
        runtime
            .remove_child(scope, runtime_ptr, parent, right)
            .then_some(())?;
    }
    Some(())
}

pub(super) fn white_space(
    runtime: &PageScriptHost,
    element: DomHandle,
) -> ComputedWhiteSpaceCollapseKind {
    ComputedStyleRead::new(runtime, element)
        .rendered_style_facts()
        .map_or(ComputedWhiteSpaceCollapseKind::Collapse, |style| {
            style.white_space_collapse
        })
}

pub(super) fn collapsed_prefix(
    runtime: &PageScriptHost,
    parent: DomHandle,
    units: &[u16],
) -> usize {
    collapsible_space_count(runtime, parent, units.iter().copied())
}

pub(super) fn collapsed_suffix(
    runtime: &PageScriptHost,
    parent: DomHandle,
    units: &[u16],
) -> usize {
    collapsible_space_count(runtime, parent, units.iter().rev().copied())
}

fn collapsible_space_count(
    runtime: &PageScriptHost,
    parent: DomHandle,
    units: impl Iterator<Item = u16>,
) -> usize {
    let collapses = white_space(runtime, parent);
    units
        .take_while(|&unit| match collapses {
            ComputedWhiteSpaceCollapseKind::Collapse | ComputedWhiteSpaceCollapseKind::Other => {
                is_collapsible_space(unit)
            }
            ComputedWhiteSpaceCollapseKind::PreserveBreaks => matches!(unit, 0x09 | 0x0c | 0x20),
            ComputedWhiteSpaceCollapseKind::Preserve
            | ComputedWhiteSpaceCollapseKind::BreakSpaces => false,
        })
        .count()
}

fn following_line_content(
    runtime: &PageScriptHost,
    host: DomHandle,
    mut container: DomHandle,
    offset: u32,
) -> Option<DomHandle> {
    let dom = runtime.dom_host();
    let mut next = dom.child_handles(container).nth(offset as usize);
    loop {
        let Some(handle) = next else {
            if container == host
                || !is_inline_container(
                    ComputedStyleRead::new(runtime, container)
                        .rendered_style_facts()?
                        .display,
                )
            {
                return None;
            }
            next = dom.next_sibling(container);
            container = dom.parent_node(container)?;
            continue;
        };
        let node = dom.node(handle)?;
        if let Some(element) = node.as_element() {
            let style = ComputedStyleRead::new(runtime, handle).rendered_style_facts()?;
            if style.display != ComputedDisplayKind::None && !style.hides_contents {
                if element.content_editable_state() == ContentEditableState::False {
                    return None;
                }
                match style.display {
                    ComputedDisplayKind::InlineAtomic | ComputedDisplayKind::InlineTable => {
                        return Some(handle);
                    }
                    ComputedDisplayKind::Inline | ComputedDisplayKind::Contents => {
                        if element.is_html_element("br")
                            || is_replaced_element(element)
                            || element.is_svg_element("svg")
                        {
                            return Some(handle);
                        }
                        container = handle;
                        next = dom.first_child(handle);
                        continue;
                    }
                    _ => return None,
                }
            }
        } else if matches!(node.node_type(), NodeType::Text | NodeType::CDataSection) {
            let data = node.character_data_value()?;
            let units = data.utf16_units();
            if collapsed_prefix(runtime, container, &units) < units.len() {
                return Some(handle);
            }
        }
        next = dom.next_sibling(handle);
    }
}

pub(super) fn is_inline_container(display: ComputedDisplayKind) -> bool {
    matches!(
        display,
        ComputedDisplayKind::Inline
            | ComputedDisplayKind::InlineAtomic
            | ComputedDisplayKind::Contents
    )
}

pub(super) fn allows_line_break(element: &Element) -> bool {
    element.namespace() == XHTML_NS
        && !matches!(
            element.local_name(),
            "area"
                | "base"
                | "br"
                | "col"
                | "colgroup"
                | "dl"
                | "embed"
                | "head"
                | "hr"
                | "html"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "ol"
                | "optgroup"
                | "option"
                | "param"
                | "script"
                | "select"
                | "source"
                | "style"
                | "table"
                | "tbody"
                | "template"
                | "textarea"
                | "tfoot"
                | "thead"
                | "title"
                | "tr"
                | "track"
                | "ul"
                | "wbr"
        )
}

pub(super) fn create_break(runtime: &mut PageScriptHost, document: DomHandle) -> DomHandle {
    runtime
        .dom_host_mut()
        .create_element_without_attributes_for_document(
            document,
            "br".to_owned(),
            XHTML_NS.to_owned(),
            None,
        )
}
