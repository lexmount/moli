use super::super::range::current_document_object;
use super::super::range_algorithms::point_order;
use crate::document_runtime::EventTargetHandle;
use crate::native_bridge::callback_value_dom_handle;
use crate::page_task_queue::RendererPageUserInteractionEventKind;
use crate::util::context_host_ptr_from_global_bridge;

const NODE_POSITION_PRECEDING: u32 = 0x02;
const NODE_POSITION_FOLLOWING: u32 = 0x04;
pub(in crate::context_bootstrap) fn selection_dispatch_change(scope: &mut v8::PinScope<'_, '_>) {
    let Some(document) = current_document_object(scope) else {
        return;
    };
    let Some(document_handle) = callback_value_dom_handle(scope, document.into()) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    if !host.has_event_listener(EventTargetHandle::Node(document_handle), "selectionchange") {
        return;
    }
    let _ = host.queue_user_interaction_event_task(
        scope,
        RendererPageUserInteractionEventKind::DocumentSelectionChange,
        document_handle,
    );
}

fn compare_document_position_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    a_node: v8::Local<'s, v8::Object>,
    b_node: v8::Local<'s, v8::Object>,
) -> u32 {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return 0;
    };
    let Some(a_handle) = callback_value_dom_handle(scope, a_node.into()) else {
        return 0;
    };
    let Some(b_handle) = callback_value_dom_handle(scope, b_node.into()) else {
        return 0;
    };
    let host = unsafe { &*host_ptr };
    host.dom_host()
        .node(a_handle)
        .map(|node| node.compare_document_position(host.dom_host().dom(), b_handle) as u32)
        .unwrap_or(0)
}

pub(in crate::context_bootstrap) fn boundary_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    a_node: v8::Local<'s, v8::Object>,
    a_offset: u32,
    b_node: v8::Local<'s, v8::Object>,
    b_offset: u32,
) -> std::cmp::Ordering {
    if let Some(order) = point_order(scope, a_node, a_offset, b_node, b_offset) {
        return order;
    }
    let bits = compare_document_position_internal(scope, a_node, b_node);
    if bits & NODE_POSITION_FOLLOWING != 0 {
        std::cmp::Ordering::Less
    } else if bits & NODE_POSITION_PRECEDING != 0 {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}
