use super::super::range::{self, RangeBoundarySide};
use super::super::range_algorithms::{
    range_delete_contents, range_extract_contents, range_insert_node_at_boundary,
};
use super::{selection_dispatch_change, selection_set_collapsed};
use crate::context_bootstrap::{live_ranges_character_data_edit, selection_value_for_window};
use crate::document_runtime::DomHandle;
use crate::dom::native::{ContentEditableState, DomHost, DomStringValue, Node, NodeType};
use crate::native_bridge::document::XHTML_NS;
use crate::native_bridge::{PageScriptHost, SelectionBoundarySnapshot};
use crate::util::node_wrapper_from_handle;

mod deletion;
mod line_break;
mod paragraph_break;
mod paragraphs;
pub(crate) use deletion::delete_editable_selection;
pub(crate) use line_break::replace_editable_selection_line_break;
pub(crate) use paragraph_break::replace_editable_selection_paragraph;
use paragraphs::normalize_fragment_for_fixed_paragraph;

pub(crate) fn replace_editable_selection_text(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    replacement: &str,
    insertion_point_is_editable: impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<bool> {
    let runtime = unsafe { &*runtime_ptr };
    let snapshot = runtime.document_editing_selection(document)?;
    let (start, end) = (snapshot.start, snapshot.end);
    let dom = runtime.dom_host();
    let order = crate::range_boundary::point_order_in_dom(
        dom,
        start.container,
        start.offset,
        end.container,
        end.offset,
    )?;
    if order.is_eq() && replacement.is_empty() {
        return Some(false);
    }
    let (start, end) = if order.is_gt() {
        (end, start)
    } else {
        (start, end)
    };

    let replacement = DomStringValue::from(replacement);
    let units = replacement.utf16_units();
    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;

    let text_span = if start.container == end.container && is_text(runtime, start.container) {
        Some((start.container, start.offset..end.offset))
    } else if !units.is_empty() && !order.is_eq() {
        let first = text_boundary(dom, start.container, start.offset, RangeBoundarySide::Start);
        let last = text_boundary(dom, end.container, end.offset, RangeBoundarySide::End);
        first.zip(last).and_then(|((first, from), (last, to))| {
            (first == last && from < to && insertion_point_is_editable(runtime, first))
                .then_some((first, from..to))
        })
    } else {
        None
    };
    let (caret, offset) = if let Some((text, offsets)) = text_span {
        replace_text_span(scope, runtime_ptr, text, offsets, &units)?
    } else {
        let range = new_editing_range(scope, document, start, end)?;
        range_delete_contents(scope, range)?;
        let start = range::native_range_boundary_handles(scope, range)?.start;
        let runtime = unsafe { &*runtime_ptr };
        // Removing selected elements can run their disconnected callbacks.
        if !insertion_point_is_editable(runtime, start.container) {
            return None;
        }
        if units.is_empty() {
            (start.container, start.offset)
        } else if let Some((text, offset)) =
            adjacent_text_insertion_point(runtime, start.container, start.offset)
        {
            replace_text_span(scope, runtime_ptr, text, offset..offset, &units)?
        } else {
            let text = unsafe { &mut *runtime_ptr }
                .create_text_node_for_document(document, replacement.clone());
            let start_node = node_wrapper_from_handle(scope, start.container)?;
            let text_node = node_wrapper_from_handle(scope, text)?;
            range_insert_node_at_boundary(scope, range, start_node, start.offset, text_node)?;
            (text, units.len() as u32)
        }
    };
    collapse_editing_selection(scope, selection, caret, offset)?;
    Some(true)
}

pub(crate) fn replace_editable_selection_fragment(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    fragment: DomHandle,
    rich_editing_host: Option<DomHandle>,
    insertion_point_is_editable: impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<()> {
    let runtime = unsafe { &*runtime_ptr };
    let snapshot = runtime.document_editing_selection(document)?;
    let dom = runtime.dom_host();
    let order = crate::range_boundary::point_order_in_dom(
        dom,
        snapshot.start.container,
        snapshot.start.offset,
        snapshot.end.container,
        snapshot.end.offset,
    )?;
    let (mut start, mut end) = if order.is_gt() {
        (snapshot.end, snapshot.start)
    } else {
        (snapshot.start, snapshot.end)
    };
    if let Some(host) = rich_editing_host {
        normalize_fragment_for_fixed_paragraph(
            unsafe { &mut *runtime_ptr },
            document,
            start.container,
            host,
            fragment,
        )?;
        if !order.is_eq() {
            let dom = unsafe { &*runtime_ptr }.dom_host();
            let block_content = fragment_has_prohibited_children(dom, fragment);
            start = expand_rich_replacement_boundary(
                dom,
                start,
                host,
                RangeBoundarySide::Start,
                block_content,
            );
            end = expand_rich_replacement_boundary(
                dom,
                end,
                host,
                RangeBoundarySide::End,
                block_content,
            );
        }
    }
    let dom = unsafe { &*runtime_ptr }.dom_host();
    let last_child = dom.child_handles_reversed(fragment).next();
    if order.is_eq() && last_child.is_none() {
        return Some(());
    }
    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let range = new_editing_range(scope, document, start, end)?;
    range_delete_contents(scope, range)?;
    let start = range::native_range_boundary_handles(scope, range)?.start;
    if !insertion_point_is_editable(unsafe { &*runtime_ptr }, start.container) {
        return None;
    }

    let (caret, offset) = if let Some(last) = last_child {
        let mut start = SelectionBoundarySnapshot {
            container: start.container,
            offset: start.offset,
        };
        if let Some(host) = rich_editing_host
            && let Some((placeholder_parent, placeholder, insertion)) =
                placeholder_for_fragment(unsafe { &*runtime_ptr }.dom_host(), host, start, fragment)
        {
            let runtime = unsafe { &mut *runtime_ptr };
            runtime
                .remove_child(scope, runtime_ptr, placeholder_parent, placeholder)
                .then_some(())?;
            start = insertion;
        }
        let runtime = unsafe { &*runtime_ptr };
        let split_ancestor = rich_editing_host.and_then(|host| {
            rich_fragment_split_ancestor(runtime.dom_host(), fragment, start.container, host)
        });
        let (insertion_node, offset) = if let Some(ancestor) = split_ancestor {
            split_editing_ancestor(
                scope,
                runtime_ptr,
                document,
                range,
                start,
                ancestor,
                &insertion_point_is_editable,
            )?
        } else {
            fragment_insertion_point(runtime, start.container, start.offset)
        };
        if !insertion_point_is_editable(unsafe { &*runtime_ptr }, insertion_node) {
            return None;
        }
        let container = node_wrapper_from_handle(scope, insertion_node)?;
        let fragment_node = node_wrapper_from_handle(scope, fragment)?;
        range_insert_node_at_boundary(scope, range, container, offset, fragment_node)?;
        let dom = unsafe { &*runtime_ptr }.dom_host();
        let parent = dom.parent_node(last)?;
        (parent, dom.child_index(parent, last)? as u32 + 1)
    } else {
        (start.container, start.offset)
    };
    if !insertion_point_is_editable(unsafe { &*runtime_ptr }, caret) {
        return None;
    }
    collapse_editing_selection(scope, selection, caret, offset)
}

fn placeholder_for_fragment(
    dom: &DomHost,
    host: DomHandle,
    start: SelectionBoundarySnapshot,
    fragment: DomHandle,
) -> Option<(DomHandle, DomHandle, SelectionBoundarySnapshot)> {
    if fragment_has_prohibited_children(dom, fragment)
        || !dom.child_handles(fragment).any(|child| {
            dom.node(child).is_some_and(|node| {
                node.is_element()
                    || node
                        .as_text()
                        .is_some_and(|text| text.data().chars().any(|ch| !ch.is_ascii_whitespace()))
            })
        })
    {
        return None;
    }
    let block = std::iter::successors(Some(start.container), |&node| dom.parent_node(node)).find(
        |&handle| {
            handle == host
                || dom.node(handle).is_some_and(|node| {
                    is_paragraph_or_heading(node) || node.is_html_element_named("div")
                })
        },
    )?;
    let line_break = lone_editor_break(dom, block)?;
    let branch = std::iter::successors(Some(line_break), |&node| dom.parent_node(node))
        .find(|&node| dom.parent_node(node) == Some(block))?;
    let branch_index = dom.child_index(block, branch)? as u32;
    if start.container == block && start.offset <= branch_index {
        // From outside the empty line's formatting, replace its whole
        // placeholder. A caret placed inside it keeps the surrounding nodes.
        return Some((block, branch, start));
    }
    let parent = dom.parent_node(line_break)?;
    let insertion = SelectionBoundarySnapshot {
        container: parent,
        offset: dom.child_index(parent, line_break)? as u32,
    };
    Some((parent, line_break, insertion))
}

fn lone_editor_break(dom: &DomHost, block: DomHandle) -> Option<DomHandle> {
    lone_editor_break_with(dom, block, |node| {
        node.as_element()
            .is_some_and(|element| element.is_html_element("span"))
    })
}

fn lone_editor_break_with(
    dom: &DomHost,
    block: DomHandle,
    is_wrapper: impl Fn(&Node) -> bool,
) -> Option<DomHandle> {
    let mut line_break = None;
    let mut pending = dom.child_handles(block).collect::<Vec<_>>();
    while let Some(child) = pending.pop() {
        let node = dom.node(child)?;
        if node.node_type() == NodeType::Comment
            || node.as_text().is_some_and(|text| text.data().is_empty())
        {
            continue;
        }
        let element = node
            .as_element()
            .filter(|element| element.attributes().is_empty())?;
        if element.is_html_element("br") && line_break.is_none() {
            line_break = Some(child);
        } else if is_wrapper(node) {
            pending.extend(dom.child_handles(child));
        } else {
            return None;
        }
    }
    line_break
}

fn expand_rich_replacement_boundary(
    dom: &DomHost,
    mut boundary: SelectionBoundarySnapshot,
    host: DomHandle,
    side: RangeBoundarySide,
    block_content: bool,
) -> SelectionBoundarySnapshot {
    let after = matches!(side, RangeBoundarySide::End);
    let mut expanded = boundary;
    while boundary.container != host {
        let Some(node) = dom.node(boundary.container) else {
            break;
        };
        let wrapper =
            is_inline_formatting(node) || (block_content && is_paragraph_or_heading(node));
        if !wrapper && !matches!(node.node_type(), NodeType::Text | NodeType::CDataSection) {
            break;
        }
        let edge = if after {
            editing_boundary_length(dom, node, boundary.container)
        } else {
            0
        };
        if boundary.offset as usize != edge {
            break;
        }
        let Some(parent) = node.parent_node_id() else {
            break;
        };
        let Some(index) = dom.child_index(parent, boundary.container) else {
            break;
        };
        boundary = SelectionBoundarySnapshot {
            container: parent,
            offset: index as u32 + u32::from(after),
        };
        // Endpoints at the edges of a formatting wrapper describe the same
        // edit from outside it. Range deletion can then remove a fully
        // selected wrapper together with its contents.
        if wrapper {
            expanded = boundary;
        }
    }
    expanded
}

fn is_paragraph_or_heading(node: &Node) -> bool {
    node.namespace() == Some(XHTML_NS)
        && matches!(
            node.local_name(),
            Some("p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
        )
}

fn fragment_has_prohibited_children(dom: &DomHost, fragment: DomHandle) -> bool {
    dom.child_handles(fragment)
        .any(|child| dom.node(child).is_some_and(is_prohibited_paragraph_child))
}

fn rich_fragment_split_ancestor(
    dom: &DomHost,
    fragment: DomHandle,
    container: DomHandle,
    host: DomHandle,
) -> Option<DomHandle> {
    if !dom
        .child_handles(fragment)
        .any(|child| dom.node(child).is_some_and(Node::is_element))
    {
        return None;
    }
    let has_block_content = fragment_has_prohibited_children(dom, fragment);
    let has_link = dom.child_handles(fragment).any(|child| {
        dom.node(child)
            .is_some_and(|node| node.is_html_element_named("a"))
    });
    let mut current = Some(container);
    let mut split = None;
    while let Some(handle) = current.filter(|handle| *handle != host) {
        let node = dom.node(handle)?;
        // The supplied fragment carries its own inline formatting. Block
        // elements also need to stay outside paragraphs and headings.
        let inline_formatting = is_inline_formatting(node);
        let carries_inline_style = inline_formatting
            && (node.local_name() != Some("span")
                || node
                    .as_element()
                    .is_some_and(|element| element.has_attribute("style")));
        if carries_inline_style
            || (has_block_content && (inline_formatting || is_paragraph_or_heading(node)))
            || (has_link && node.is_html_element_named("a"))
        {
            split = Some(handle);
        }
        current = node.parent_node_id();
    }
    split
}

fn is_prohibited_paragraph_child(node: &Node) -> bool {
    node.namespace() == Some(XHTML_NS)
        && matches!(
            node.local_name(),
            Some(
                "address"
                    | "article"
                    | "aside"
                    | "blockquote"
                    | "caption"
                    | "center"
                    | "col"
                    | "colgroup"
                    | "dd"
                    | "details"
                    | "dir"
                    | "div"
                    | "dl"
                    | "dt"
                    | "fieldset"
                    | "figcaption"
                    | "figure"
                    | "footer"
                    | "form"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "header"
                    | "hgroup"
                    | "hr"
                    | "li"
                    | "listing"
                    | "menu"
                    | "nav"
                    | "ol"
                    | "p"
                    | "plaintext"
                    | "pre"
                    | "search"
                    | "section"
                    | "summary"
                    | "table"
                    | "tbody"
                    | "td"
                    | "tfoot"
                    | "th"
                    | "thead"
                    | "tr"
                    | "ul"
                    | "xmp"
            )
        )
}

fn split_editing_ancestor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    range: v8::Local<'s, v8::Object>,
    start: SelectionBoundarySnapshot,
    ancestor: DomHandle,
    insertion_point_is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<(DomHandle, u32)> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let parent = dom.parent_node(ancestor)?;
    let index = dom.child_index(parent, ancestor)? as u32;
    if let Some(after) = boundary_at_ancestor_edge(dom, start, ancestor) {
        return Some((parent, index + u32::from(after)));
    }

    let end = SelectionBoundarySnapshot {
        container: ancestor,
        offset: dom.child_handles(ancestor).count() as u32,
    };
    let trailing_range = new_editing_range(scope, document, start, end)?;
    let runtime = unsafe { &mut *runtime_ptr };
    let continuation = runtime.clone_node(scope, runtime_ptr, ancestor, false)?;
    runtime.dom_host_mut().remove_attribute(continuation, "id");
    let tail = range_extract_contents(scope, trailing_range)?;

    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    if dom.parent_node(ancestor) != Some(parent) || !insertion_point_is_editable(runtime, parent) {
        // A disconnected callback can move the original paragraph during
        // extraction. Return the unselected tail before aborting the paste.
        let offset = dom.child_handles(ancestor).count() as u32;
        let original = node_wrapper_from_handle(scope, ancestor)?;
        range_insert_node_at_boundary(scope, trailing_range, original, offset, tail)?;
        return None;
    }
    let offset = dom.child_index(parent, ancestor)? as u32 + 1;
    let continuation_node = node_wrapper_from_handle(scope, continuation)?;
    range_insert_node_at_boundary(scope, trailing_range, continuation_node, 0, tail)?;
    let parent_node = node_wrapper_from_handle(scope, parent)?;
    range_insert_node_at_boundary(scope, range, parent_node, offset, continuation_node)?;
    let dom = unsafe { &*runtime_ptr }.dom_host();
    Some((parent, dom.child_index(parent, continuation)? as u32))
}

fn boundary_at_ancestor_edge(
    dom: &DomHost,
    mut boundary: SelectionBoundarySnapshot,
    ancestor: DomHandle,
) -> Option<bool> {
    loop {
        let node = dom.node(boundary.container)?;
        let length = editing_boundary_length(dom, node, boundary.container);
        let after = boundary.offset != 0;
        if after && boundary.offset as usize != length {
            return None;
        }
        if boundary.container == ancestor {
            return Some(after);
        }
        let parent = node.parent_node_id()?;
        let index = dom.child_index(parent, boundary.container)? as u32;
        boundary = SelectionBoundarySnapshot {
            container: parent,
            offset: index + u32::from(after),
        };
    }
}

fn editing_boundary_length(dom: &DomHost, node: &Node, handle: DomHandle) -> usize {
    node.character_data_value().map_or_else(
        || dom.child_handles(handle).count(),
        |data| data.utf16_units().len(),
    )
}

fn new_editing_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: DomHandle,
    start: SelectionBoundarySnapshot,
    end: SelectionBoundarySnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    let document = node_wrapper_from_handle(scope, document)?;
    let start_node = node_wrapper_from_handle(scope, start.container)?;
    let end_node = node_wrapper_from_handle(scope, end.container)?;
    let range = range::new_range_for_document(scope, document)?;
    range::set_range_boundary(
        scope,
        range,
        RangeBoundarySide::Start,
        start_node,
        start.offset,
    );
    range::set_range_boundary(scope, range, RangeBoundarySide::End, end_node, end.offset);
    Some(range)
}

fn collapse_editing_selection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    container: DomHandle,
    offset: u32,
) -> Option<()> {
    let caret = node_wrapper_from_handle(scope, container)?;
    selection_set_collapsed(scope, selection, caret, offset).then_some(())?;
    selection_dispatch_change(scope);
    Some(())
}

fn fragment_insertion_point(
    runtime: &PageScriptHost,
    container: DomHandle,
    offset: u32,
) -> (DomHandle, u32) {
    // At the edge of a text node, insert beside it instead of splitting off
    // an empty text node.
    let dom = runtime.dom_host();
    if is_text(runtime, container)
        && let Some(data) = dom.node(container).and_then(Node::character_data_value)
        && (offset == 0 || offset as usize == data.utf16_units().len())
        && let Some(parent) = dom.parent_node(container)
        && let Some(index) = dom.child_index(parent, container)
    {
        return (parent, index as u32 + u32::from(offset != 0));
    }
    (container, offset)
}

fn is_text(runtime: &PageScriptHost, node: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(node)
        .is_some_and(|node| matches!(node.node_type(), NodeType::Text | NodeType::CDataSection))
}

fn adjacent_text_insertion_point(
    runtime: &PageScriptHost,
    container: DomHandle,
    offset: u32,
) -> Option<(DomHandle, u32)> {
    if is_text(runtime, container) {
        return Some((container, offset));
    }
    let dom = runtime.dom_host();
    let mut children = dom.child_handles(container);
    let previous = offset
        .checked_sub(1)
        .and_then(|index| children.nth(index as usize));
    if let Some(next) = children.next().filter(|child| is_text(runtime, *child)) {
        return Some((next, 0));
    }
    let previous = previous.filter(|child| is_text(runtime, *child))?;
    let length = dom
        .node(previous)?
        .character_data_value()?
        .utf16_units()
        .len();
    Some((previous, length as u32))
}

// A boundary just outside inline formatting can select the same text as
// a boundary within it. Reuse the text node when both boundaries select a
// single span so replacing that text preserves its inline formatting.
fn text_boundary(
    dom: &DomHost,
    mut node: DomHandle,
    mut offset: u32,
    side: RangeBoundarySide,
) -> Option<(DomHandle, u32)> {
    let forward = matches!(side, RangeBoundarySide::Start);
    loop {
        let current = dom.node(node)?;
        if matches!(current.node_type(), NodeType::Text | NodeType::CDataSection) {
            let length = current.character_data_value()?.utf16_units().len() as u32;
            if (forward && offset < length) || (!forward && offset != 0) {
                return Some((node, offset));
            }
        } else {
            let index = if forward {
                Some(offset)
            } else {
                offset.checked_sub(1)
            };
            if let Some(child) = index.and_then(|index| dom.child_handles(node).nth(index as usize))
            {
                let candidate = dom.node(child)?;
                if !is_inline_formatting(candidate)
                    && !matches!(
                        candidate.node_type(),
                        NodeType::Text | NodeType::CDataSection
                    )
                {
                    return None;
                }
                node = child;
                offset = if forward {
                    0
                } else if let Some(data) = candidate.character_data_value() {
                    data.utf16_units().len() as u32
                } else {
                    dom.child_handles(node).count() as u32
                };
                continue;
            }
            if !is_inline_formatting(current) {
                return None;
            }
        }
        let parent = current.parent_node_id()?;
        let index = dom.child_index(parent, node)? as u32;
        node = parent;
        offset = index + u32::from(forward);
    }
}

fn is_inline_formatting(node: &Node) -> bool {
    node.as_element().is_some_and(|element| {
        element.namespace() == XHTML_NS
            && element.content_editable_state() == ContentEditableState::Inherit
            && !element.has_attribute("inert")
            && matches!(
                element.local_name(),
                "b" | "big"
                    | "code"
                    | "em"
                    | "font"
                    | "i"
                    | "mark"
                    | "s"
                    | "small"
                    | "span"
                    | "strike"
                    | "strong"
                    | "sub"
                    | "sup"
                    | "tt"
                    | "u"
            )
    })
}

fn replace_text_span(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    offsets: std::ops::Range<u32>,
    replacement: &[u16],
) -> Option<(DomHandle, u32)> {
    let runtime = unsafe { &mut *runtime_ptr };
    let mut units = runtime.character_data_utf16_units(node)?;
    if offsets.end as usize > units.len() {
        return None;
    }
    units.splice(
        offsets.start as usize..offsets.end as usize,
        replacement.iter().copied(),
    );
    // An identical replacement still moves the live ranges and the caret
    // even when the data setter reports no observable mutation.
    runtime.set_character_data_utf16_units_for_edit(scope, runtime_ptr, node, &units);
    let inserted = replacement.len() as u32;
    live_ranges_character_data_edit(
        scope,
        node,
        offsets.start,
        offsets.end - offsets.start,
        inserted,
    );
    Some((node, offsets.start.saturating_add(inserted)))
}
