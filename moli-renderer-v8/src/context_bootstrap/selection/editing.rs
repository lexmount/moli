use super::super::range::{self, RangeBoundarySide};
use super::super::range_algorithms::{range_delete_contents, range_insert_node_at_boundary};
use super::{selection_dispatch_change, selection_set_collapsed};
use crate::context_bootstrap::{live_ranges_character_data_edit, selection_value_for_window};
use crate::document_runtime::DomHandle;
use crate::dom::native::{DomStringValue, NodeType};
use crate::native_bridge::PageScriptHost;
use crate::util::node_wrapper_from_handle;

pub(crate) fn replace_editable_selection_text(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    replacement: &str,
    insertion_point_is_editable: impl FnOnce(&PageScriptHost, DomHandle) -> bool,
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

    let (caret, offset) = if start.container == end.container && is_text(runtime, start.container) {
        replace_text_span(
            scope,
            runtime_ptr,
            start.container,
            start.offset..end.offset,
            &units,
        )?
    } else {
        let document_object = node_wrapper_from_handle(scope, document)?;
        let start_node = node_wrapper_from_handle(scope, start.container)?;
        let end_node = node_wrapper_from_handle(scope, end.container)?;
        let range = range::new_range_for_document(scope, document_object)?;
        range::set_range_boundary(
            scope,
            range,
            RangeBoundarySide::Start,
            start_node,
            start.offset,
        );
        range::set_range_boundary(scope, range, RangeBoundarySide::End, end_node, end.offset);

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
    let caret = node_wrapper_from_handle(scope, caret)?;
    selection_set_collapsed(scope, selection, caret, offset).then_some(())?;
    selection_dispatch_change(scope);
    Some(true)
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
