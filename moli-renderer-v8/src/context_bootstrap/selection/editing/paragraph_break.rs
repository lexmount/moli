use super::line_break::{
    BreakInsertion, allows_line_break, collapsed_prefix, collapsed_suffix, create_break,
    insert_line_break_at_range, is_inline_container, prepare_break,
};
use super::{
    boundary_at_ancestor_edge, collapse_editing_selection, editing_boundary_length,
    is_inline_formatting, is_paragraph_or_heading, is_prohibited_paragraph_child,
    lone_editor_break, new_editing_range, range, range_extract_contents,
    range_insert_node_at_boundary, replace_text_span, split_editing_ancestor,
};
use crate::document_runtime::DomHandle;
use crate::dom::native::{ContentEditableState, DomHost, Node, NodeType};
use crate::native_bridge::document::XHTML_NS;
use crate::native_bridge::{PageScriptHost, SelectionBoundarySnapshot, element::ComputedStyleRead};
use crate::style_engine::ComputedDisplayKind;
use crate::util::node_wrapper_from_handle;

enum ParagraphContainer {
    Block(DomHandle),
    InlineRun(DomHandle),
    LineBreak,
}

struct ParagraphInsertion<'s> {
    document: DomHandle,
    separator: &'static str,
    edit: BreakInsertion<'s>,
}

pub(crate) fn replace_editable_selection_paragraph(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    is_editable: impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<bool> {
    let edit = prepare_break(scope, runtime_ptr, document, host, &is_editable)?;
    let start = range::native_range_boundary_handles(scope, edit.range)?.start;
    let point = SelectionBoundarySnapshot {
        container: start.container,
        offset: start.offset,
    };
    let runtime = unsafe { &*runtime_ptr };
    if !is_editable(runtime, point.container) {
        return None;
    }
    let Some(container) = paragraph_container(runtime, host, point.container) else {
        return Some(edit.deleted_selection);
    };
    let insertion = ParagraphInsertion {
        document,
        separator: runtime
            .dom_host()
            .node(document)?
            .as_document()?
            .paragraph_separator()
            .tag_name(),
        edit,
    };
    let (block, point) = match container {
        ParagraphContainer::Block(block) => (block, point),
        ParagraphContainer::InlineRun(parent) => {
            wrap_inline_run(scope, runtime_ptr, &insertion, parent, point, &is_editable)?
        }
        ParagraphContainer::LineBreak => {
            return insert_line_break_at_range(
                scope,
                runtime_ptr,
                document,
                host,
                insertion.edit,
                &is_editable,
                true,
            );
        }
    };
    let runtime = unsafe { &*runtime_ptr };
    let caret = if let Some(list) = empty_list_item_parent(runtime, block, &is_editable) {
        if runtime
            .dom_host()
            .node(list)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                matches!(
                    element.content_editable_state(),
                    ContentEditableState::True | ContentEditableState::PlaintextOnly
                )
            })
        {
            // An explicitly editable list owns its list items.
            return Some(insertion.edit.deleted_selection);
        }
        let paragraph =
            exit_empty_list_item(scope, runtime_ptr, &insertion, list, block, &is_editable)?;
        SelectionBoundarySnapshot {
            container: paragraph,
            offset: 0,
        }
    } else {
        split_paragraph(scope, runtime_ptr, &insertion, block, point, &is_editable)?
    };
    collapse_editing_selection(
        scope,
        insertion.edit.selection,
        caret.container,
        caret.offset,
    )?;
    Some(true)
}

fn paragraph_container(
    runtime: &PageScriptHost,
    host: DomHandle,
    node: DomHandle,
) -> Option<ParagraphContainer> {
    let dom = runtime.dom_host();
    let mut block = None;
    for handle in std::iter::successors(Some(node), |&handle| dom.parent_node(handle)) {
        let Some(element) = dom.node(handle)?.as_element() else {
            continue;
        };
        let style = ComputedStyleRead::new(runtime, handle).rendered_style_facts()?;
        if style.display == ComputedDisplayKind::None || style.hides_contents {
            return None;
        }
        let html = element.namespace() == XHTML_NS;
        let tag = element.local_name();
        if handle == host {
            return block.map(ParagraphContainer::Block).or_else(|| {
                if is_inline_container(style.display)
                    || html
                        && matches!(
                            tag,
                            "p" | "pre"
                                | "dt"
                                | "address"
                                | "listing"
                                | "h1"
                                | "h2"
                                | "h3"
                                | "h4"
                                | "h5"
                                | "h6"
                        )
                {
                    Some(ParagraphContainer::LineBreak)
                } else if allows_line_break(element) {
                    Some(ParagraphContainer::InlineRun(host))
                } else {
                    None
                }
            });
        }
        if html && matches!(tag, "li" | "dt" | "dd") {
            return Some(ParagraphContainer::Block(handle));
        }
        match style.display {
            ComputedDisplayKind::TableCell => {
                return Some(block.map_or(
                    ParagraphContainer::InlineRun(handle),
                    ParagraphContainer::Block,
                ));
            }
            ComputedDisplayKind::Table
            | ComputedDisplayKind::InlineTable
            | ComputedDisplayKind::TableInternal
            | ComputedDisplayKind::TableRow => return block.map(ParagraphContainer::Block),
            _ => {}
        }
        if block.is_none() && !is_inline_container(style.display) {
            if html && matches!(tag, "address" | "listing" | "pre") {
                return Some(ParagraphContainer::LineBreak);
            }
            block = Some(handle);
        }
    }
    None
}

fn wrap_inline_run<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    insertion: &ParagraphInsertion<'s>,
    parent: DomHandle,
    point: SelectionBoundarySnapshot,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<(DomHandle, SelectionBoundarySnapshot)> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let point = lift_split_point(dom, point, parent);
    let children = dom.child_handles(parent).collect::<Vec<_>>();
    let (before, after) = if point.container == parent {
        (point.offset as usize, point.offset as usize)
    } else {
        let child = std::iter::successors(Some(point.container), |&node| dom.parent_node(node))
            .find(|&node| dom.parent_node(node) == Some(parent))?;
        let index = dom.child_index(parent, child)?;
        (index, index + 1)
    };
    let ends_run = |child, stop_at_break| {
        dom.node(child).is_none_or(|node| {
            is_prohibited_paragraph_child(node)
                || stop_at_break && node.is_html_element_named("br")
                || node.is_element()
                    && ComputedStyleRead::new(runtime, child)
                        .rendered_style_facts()
                        .is_some_and(|style| {
                            style.display != ComputedDisplayKind::None
                                && !is_inline_container(style.display)
                        })
        })
    };
    let first = before
        - children
            .get(..before)?
            .iter()
            .rev()
            .take_while(|&&child| !ends_run(child, true))
            .count();
    let end = after
        + children
            .get(after..)?
            .iter()
            .take_while(|&&child| !ends_run(child, false))
            .count();
    if before != after && ends_run(children[before], false) {
        return None;
    }
    let start = SelectionBoundarySnapshot {
        container: parent,
        offset: first as u32,
    };
    let end = SelectionBoundarySnapshot {
        container: parent,
        offset: end as u32,
    };
    let run = new_editing_range(scope, insertion.document, start, end)?;
    let content = range_extract_contents(scope, run)?;
    let runtime = unsafe { &mut *runtime_ptr };
    let start = range::native_range_boundary_handles(scope, run)?.start;
    if start.container != parent || !is_editable(runtime, parent) {
        let offset = (first as u32).min(runtime.dom_host().child_handles(parent).count() as u32);
        let parent_node = node_wrapper_from_handle(scope, parent)?;
        range_insert_node_at_boundary(scope, run, parent_node, offset, content)?;
        return None;
    }
    let block = create_paragraph(runtime, insertion, insertion.separator);
    let block_node = node_wrapper_from_handle(scope, block)?;
    range_insert_node_at_boundary(scope, run, block_node, 0, content)?;
    insert_node(scope, insertion.edit.range, parent, start.offset, block)?;
    let point = if point.container == parent {
        SelectionBoundarySnapshot {
            container: block,
            offset: point.offset - first as u32,
        }
    } else {
        point
    };
    Some((block, point))
}

fn split_paragraph<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    insertion: &ParagraphInsertion<'s>,
    block: DomHandle,
    point: SelectionBoundarySnapshot,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<SelectionBoundarySnapshot> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let parent = dom.parent_node(block)?;
    if !is_editable(runtime, block)
        || !is_editable(runtime, parent)
        || !is_editable(runtime, point.container)
        || !std::iter::successors(Some(point.container), |&node| dom.parent_node(node))
            .any(|node| node == block)
    {
        return None;
    }
    let start = lift_split_point(dom, point, block);
    let end = SelectionBoundarySnapshot {
        container: block,
        offset: dom.child_handles(block).count() as u32,
    };
    let at_end = boundary_at_ancestor_edge(dom, start, block) == Some(true)
        || start.container == block
            && start.offset + 1 == end.offset
            && dom
                .last_child(block)
                .and_then(|last| dom.node(last))
                .is_some_and(|node| node.is_html_element_named("br"));
    let element = dom.node(block)?.as_element()?;
    let new_tag =
        if at_end && is_paragraph_or_heading(dom.node(block)?) && !element.is_html_element("p") {
            Some(insertion.separator)
        } else if at_end && element.is_html_element("dt") {
            Some("dd")
        } else if at_end && element.is_html_element("dd") {
            Some("dt")
        } else {
            None
        };
    let attributes = new_tag.map(|_| element.attributes().to_vec());
    let cloned_wrappers =
        std::iter::successors(Some(start.container), |&node| dom.parent_node(node))
            .take_while(|&node| node != block)
            .filter(|&node| dom.node(node).is_some_and(Node::is_element))
            .map(|node| (node, dom.next_sibling(node)))
            .collect::<Vec<_>>();
    let mut formatting = Vec::new();
    if at_end {
        for handle in std::iter::successors(Some(point.container), |&node| dom.parent_node(node))
            .take_while(|&node| node != block)
        {
            let node = dom.node(handle)?;
            if node.as_element().is_some_and(|element| {
                element.is_html_element("a") && element.has_attribute("href")
            }) {
                // The new paragraph starts outside this link and its formatting.
                formatting.clear();
            } else if is_inline_formatting(node) {
                formatting.push(handle);
            }
        }
    }
    let trailing = new_editing_range(scope, insertion.document, start, end)?;
    let runtime = unsafe { &mut *runtime_ptr };
    let continuation = if let Some((tag, attributes)) = new_tag.zip(attributes) {
        let new_block = create_paragraph(runtime, insertion, tag);
        runtime
            .dom_host_mut()
            .add_attrs_if_missing_for_parser(new_block, attributes);
        new_block
    } else {
        runtime.clone_node(scope, runtime_ptr, block, false)?
    };
    runtime.dom_host_mut().remove_attribute(continuation, "id");
    let tail = range_extract_contents(scope, trailing)?;
    let runtime = unsafe { &mut *runtime_ptr };
    let dom = runtime.dom_host();
    if dom.parent_node(block) != Some(parent) || !is_editable(runtime, parent) {
        let offset = dom.child_handles(block).count() as u32;
        let block_node = node_wrapper_from_handle(scope, block)?;
        range_insert_node_at_boundary(scope, trailing, block_node, offset, tail)?;
        return None;
    }
    let continuation_node = node_wrapper_from_handle(scope, continuation)?;
    range_insert_node_at_boundary(scope, trailing, continuation_node, 0, tail)?;
    // Partially selected wrappers were cloned. Keep IDs on nodes that moved
    // with their original identities and remove IDs from just those clones.
    let mut node = continuation;
    for (original, following) in cloned_wrappers.into_iter().rev() {
        let runtime = unsafe { &mut *runtime_ptr };
        let child = match runtime.dom_host().first_child(node) {
            Some(child) if child != original && Some(child) != following => {
                runtime.dom_host_mut().remove_attribute(child, "id");
                child
            }
            None if at_end => {
                clone_wrapper(scope, runtime_ptr, insertion.edit.range, node, original)?
            }
            _ => break,
        };
        node = child;
    }
    if at_end
        && unsafe { &*runtime_ptr }
            .dom_host()
            .first_child(node)
            .is_none()
    {
        for original in formatting.into_iter().rev() {
            node = clone_wrapper(scope, runtime_ptr, insertion.edit.range, node, original)?;
        }
    }
    let runtime = unsafe { &*runtime_ptr };
    let offset = runtime.dom_host().child_index(parent, block)? as u32 + 1;
    insert_node(scope, insertion.edit.range, parent, offset, continuation)?;
    let left_empty = pad_empty_paragraph(scope, runtime_ptr, insertion, block, is_editable)?;
    let right_empty =
        pad_empty_paragraph(scope, runtime_ptr, insertion, continuation, is_editable)?;
    if !left_empty && !right_empty {
        preserve_trailing_space(scope, runtime_ptr, block, is_editable)?;
    }
    let runtime = unsafe { &*runtime_ptr };
    let mut caret = SelectionBoundarySnapshot {
        container: continuation,
        offset: 0,
    };
    while let Some((index, child)) = runtime
        .dom_host()
        .child_handles(caret.container)
        .enumerate()
        .find(|&(_, child)| !is_empty_node(runtime.dom_host(), child))
    {
        caret.offset = index as u32;
        let node = runtime.dom_host().node(child)?;
        if let Some(data) = node.character_data_value()
            && matches!(node.node_type(), NodeType::Text | NodeType::CDataSection)
            && is_editable(runtime, child)
        {
            let data = data.utf16_units();
            let prefix = collapsed_prefix(runtime, caret.container, &data);
            if !left_empty && prefix != 0 && prefix < data.len() {
                replace_text_span(scope, runtime_ptr, child, 0..prefix as u32, &[0xa0])?;
            }
            return Some(SelectionBoundarySnapshot {
                container: child,
                offset: 0,
            });
        }
        if !node.as_element().is_some_and(allows_line_break) || !is_editable(runtime, child) {
            break;
        }
        caret = SelectionBoundarySnapshot {
            container: child,
            offset: 0,
        };
    }
    Some(caret)
}

fn clone_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    range: v8::Local<'s, v8::Object>,
    parent: DomHandle,
    original: DomHandle,
) -> Option<DomHandle> {
    let runtime = unsafe { &mut *runtime_ptr };
    let clone = runtime.clone_node(scope, runtime_ptr, original, false)?;
    unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .remove_attribute(clone, "id");
    insert_node(scope, range, parent, 0, clone)?;
    Some(clone)
}

fn preserve_trailing_space(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    mut block: DomHandle,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<()> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    while let Some(child) = dom
        .child_handles_reversed(block)
        .find(|&child| !is_empty_node(dom, child))
    {
        let node = dom.node(child)?;
        if !is_editable(runtime, child) {
            break;
        }
        if matches!(node.node_type(), NodeType::Text | NodeType::CDataSection)
            && let Some(text) = node.character_data_value()
        {
            let units = text.utf16_units();
            let count = collapsed_suffix(runtime, block, &units);
            if count != 0 {
                replace_text_span(
                    scope,
                    runtime_ptr,
                    child,
                    (units.len() - count) as u32..units.len() as u32,
                    &[0xa0],
                )?;
            }
            break;
        }
        if !node.as_element().is_some_and(allows_line_break) {
            break;
        }
        block = child;
    }
    Some(())
}

fn is_empty_node(dom: &DomHost, child: DomHandle) -> bool {
    dom.node(child).is_some_and(|node| {
        node.node_type() == NodeType::Comment
            || node.as_text().is_some_and(|text| text.data().is_empty())
    })
}

fn lift_split_point(
    dom: &DomHost,
    mut point: SelectionBoundarySnapshot,
    ancestor: DomHandle,
) -> SelectionBoundarySnapshot {
    while point.container != ancestor {
        let Some(node) = dom.node(point.container) else {
            break;
        };
        if is_prohibited_paragraph_child(node) {
            break;
        }
        let after = point.offset != 0;
        if after && point.offset as usize != editing_boundary_length(dom, node, point.container) {
            break;
        }
        let Some(parent) = dom.parent_node(point.container) else {
            break;
        };
        let Some(index) = dom.child_index(parent, point.container) else {
            break;
        };
        point = SelectionBoundarySnapshot {
            container: parent,
            offset: index as u32 + u32::from(after),
        };
    }
    point
}

fn pad_empty_paragraph<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    insertion: &ParagraphInsertion<'s>,
    mut block: DomHandle,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<bool> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    while let Some(child) = dom
        .child_handles_reversed(block)
        .find(|&child| !is_empty_node(dom, child))
        .filter(|&child| {
            is_editable(runtime, child)
                && dom.node(child).is_some_and(|node| {
                    node.is_html_element_named("p")
                        || node.is_html_element_named("div")
                        || is_inline_formatting(node)
                            && dom
                                .child_handles(block)
                                .all(|sibling| sibling == child || is_empty_node(dom, sibling))
                })
        })
    {
        block = child;
    }
    if !is_editable(runtime, block) {
        return None;
    }
    let mut blank_text = Vec::new();
    let mut has_break = false;
    for child in dom.child_handles(block) {
        let node = dom.node(child)?;
        if matches!(node.node_type(), NodeType::Text | NodeType::CDataSection)
            && let Some(text) = node.character_data_value()
        {
            let data = text.utf16_units();
            if collapsed_prefix(runtime, block, &data) == data.len() {
                blank_text.push(child);
                continue;
            }
        }
        if node.node_type() == NodeType::Comment {
            continue;
        }
        if node.is_html_element_named("br") && !has_break {
            has_break = true;
            continue;
        }
        return Some(false);
    }
    for text in blank_text {
        unsafe { &mut *runtime_ptr }
            .remove_child(scope, runtime_ptr, block, text)
            .then_some(())?;
    }
    if !has_break {
        let runtime = unsafe { &mut *runtime_ptr };
        let offset = runtime.dom_host().child_handles(block).count() as u32;
        let padding = create_break(runtime, insertion.document);
        insert_node(scope, insertion.edit.range, block, offset, padding)?;
    }
    Some(true)
}

fn empty_list_item_parent(
    runtime: &PageScriptHost,
    item: DomHandle,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<DomHandle> {
    let dom = runtime.dom_host();
    let item_node = dom.node(item)?.as_element()?;
    let parent = dom.parent_node(item)?;
    let list = dom.node(parent)?.as_element()?;
    let item_in_list = (item_node.is_html_element("li")
        && (list.is_html_element("ol") || list.is_html_element("ul")))
        || ((item_node.is_html_element("dt") || item_node.is_html_element("dd"))
            && list.is_html_element("dl"));
    (item_in_list
        && is_editable(runtime, parent)
        && (dom
            .child_handles(item)
            .all(|child| is_empty_node(dom, child))
            || lone_editor_break(dom, item).is_some()))
    .then_some(parent)
}

fn exit_empty_list_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    insertion: &ParagraphInsertion<'s>,
    list: DomHandle,
    item: DomHandle,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<DomHandle> {
    let runtime = unsafe { &mut *runtime_ptr };
    let parent = runtime.dom_host().parent_node(list)?;
    let offset = runtime.dom_host().child_index(list, item)? as u32;
    if !is_editable(runtime, parent) {
        return None;
    }
    runtime
        .remove_child(scope, runtime_ptr, list, item)
        .then_some(())?;
    let runtime = unsafe { &mut *runtime_ptr };
    let dom = runtime.dom_host();
    if dom.parent_node(list) != Some(parent) || !is_editable(runtime, parent) {
        return None;
    }
    if dom
        .node(parent)
        .is_some_and(|node| node.is_html_element_named("li"))
        && dom
            .node(item)
            .is_some_and(|node| node.is_html_element_named("li"))
        && let Some(outer) = dom.parent_node(parent)
        && dom.node(outer).is_some_and(|node| {
            node.is_html_element_named("ol") || node.is_html_element_named("ul")
        })
        && is_editable(runtime, outer)
    {
        return outdent_empty_list_item(
            scope,
            runtime_ptr,
            insertion,
            item,
            (list, offset),
            parent,
            is_editable,
        );
    }
    let (parent, offset) = if dom.first_child(list).is_none() {
        let offset = runtime.dom_host().child_index(parent, list)? as u32;
        runtime
            .remove_child(scope, runtime_ptr, parent, list)
            .then_some(())?;
        (parent, offset)
    } else {
        split_editing_ancestor(
            scope,
            runtime_ptr,
            insertion.document,
            insertion.edit.range,
            SelectionBoundarySnapshot {
                container: list,
                offset,
            },
            list,
            is_editable,
        )?
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if !is_editable(runtime, parent) {
        return None;
    }
    let next = create_paragraph(runtime, insertion, insertion.separator);
    let padding = create_break(runtime, insertion.document);
    insert_node(scope, insertion.edit.range, next, 0, padding)?;
    insert_node(scope, insertion.edit.range, parent, offset, next)?;
    is_editable(unsafe { &*runtime_ptr }, next).then_some(next)
}

fn outdent_empty_list_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    insertion: &ParagraphInsertion<'s>,
    item: DomHandle,
    (list, offset): (DomHandle, u32),
    parent: DomHandle,
    is_editable: &impl Fn(&PageScriptHost, DomHandle) -> bool,
) -> Option<DomHandle> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host();
    let outer = dom.parent_node(parent)?;
    let count = dom.child_handles(list).count() as u32;
    let tail = if offset == 0 && count != 0 {
        // The promoted item takes the entire remaining list.
        Some(list)
    } else if offset < count {
        let (parent, continuation_offset) = split_editing_ancestor(
            scope,
            runtime_ptr,
            insertion.document,
            insertion.edit.range,
            SelectionBoundarySnapshot {
                container: list,
                offset,
            },
            list,
            is_editable,
        )?;
        unsafe { &*runtime_ptr }
            .dom_host()
            .child_handles(parent)
            .nth(continuation_offset as usize)
    } else {
        None
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let dom = runtime.dom_host();
    if dom.parent_node(parent) != Some(outer) || !is_editable(runtime, outer) {
        return None;
    }
    let after_parent = dom.child_index(outer, parent)? as u32 + 1;
    if lone_editor_break(dom, item).is_none() {
        let padding = create_break(runtime, insertion.document);
        insert_node(scope, insertion.edit.range, item, 0, padding)?;
    }
    insert_node(scope, insertion.edit.range, outer, after_parent, item)?;
    let runtime = unsafe { &mut *runtime_ptr };
    if !is_editable(runtime, item) {
        return None;
    }
    if let Some(tail) = tail.filter(|&tail| runtime.dom_host().parent_node(tail) == Some(parent)) {
        runtime
            .append_child(scope, runtime_ptr, item, tail)
            .then_some(())?;
    } else if runtime.dom_host().parent_node(list) == Some(parent)
        && runtime.dom_host().first_child(list).is_none()
    {
        runtime
            .remove_child(scope, runtime_ptr, parent, list)
            .then_some(())?;
    }
    Some(item)
}

fn create_paragraph(
    runtime: &mut PageScriptHost,
    insertion: &ParagraphInsertion<'_>,
    tag: &str,
) -> DomHandle {
    runtime
        .dom_host_mut()
        .create_element_without_attributes_for_document(
            insertion.document,
            tag.to_owned(),
            XHTML_NS.to_owned(),
            None,
        )
}

fn insert_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    parent: DomHandle,
    offset: u32,
    node: DomHandle,
) -> Option<()> {
    let parent = node_wrapper_from_handle(scope, parent)?;
    let node = node_wrapper_from_handle(scope, node)?;
    range_insert_node_at_boundary(scope, range, parent, offset, node)
}
