use super::*;
use crate::dom::native::DomStringValue;
use crate::util::utf16_slice_lossy;

pub(in crate::context_bootstrap) fn create_contextual_fragment_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    markup: &DomStringValue,
) -> Option<v8::Local<'s, v8::Object>> {
    let start = native_range_boundary_point(scope, receiver, RangeBoundarySide::Start)?.container();
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &mut *host_ptr };
    let document_handle = runtime.dom_host().owner_document_handle(start)?;
    let element = match runtime.dom_host().node(start)?.node_type() {
        NodeType::Element => Some(start),
        NodeType::Text | NodeType::CDataSection | NodeType::Comment => {
            runtime.dom_host().parent_node(start).filter(|parent| {
                runtime
                    .dom_host()
                    .node(*parent)
                    .is_some_and(|node| node.is_element())
            })
        }
        _ => None,
    };
    let element = element.filter(|element| {
        !(runtime
            .dom_host()
            .node_document_is_html_document(*element)
            .unwrap_or(false)
            && runtime
                .dom_host()
                .node(*element)
                .is_some_and(|node| node.is_html_element_named("html")))
    });
    // Context selection uses the actual tree, including for windowless documents
    // and registered native Proxies. Public Node properties are author-controlled.
    let context_handle = match element {
        Some(element) => element,
        None => runtime.create_fragment_body_context(document_handle),
    };
    let fragment = match runtime.build_range_contextual_fragment(
        scope,
        host_ptr,
        document_handle,
        context_handle,
        markup,
    ) {
        Ok(Some(fragment)) => fragment,
        Ok(None) => return None,
        Err(error) => {
            crate::context_bootstrap::throw_dom_exception_value(
                scope,
                &error.to_string(),
                "SyntaxError",
            );
            return None;
        }
    };
    runtime
        .native_bridge_mut()
        .wrap_handle_for_receiver(scope, host_ptr, receiver, fragment)
}

pub(in crate::context_bootstrap::range_algorithms) fn create_document_fragment_handle(
    scope: &mut v8::PinScope<'_, '_>,
    document_handle: Option<DomHandle>,
) -> Option<DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    Some(match document_handle {
        Some(document_handle) => {
            unsafe { &mut *host_ptr }.create_document_fragment_for_document(document_handle)
        }
        None => unsafe { &mut *host_ptr }.create_document_fragment(),
    })
}

pub(in crate::context_bootstrap::range_algorithms) fn node_wrapper_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    node_wrapper_from_handle(scope, handle)
}

pub(in crate::context_bootstrap::range_algorithms) fn node_wrapper_for_handle_prefer_paired<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    crate::native_bridge::wrapped_handle_value(scope, host_ptr, handle)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap::range_algorithms) fn document_handle_for_node_handle_or_self(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }
        .dom_host()
        .owner_document_handle(handle)
}

pub(in crate::context_bootstrap::range_algorithms) fn clone_node_internal_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
    deep: bool,
) -> Option<DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    clone_node_internal_handle_with_host(scope, host_ptr, handle, deep)
}

pub(in crate::context_bootstrap::range_algorithms) fn split_text_internal_handle(
    scope: &mut v8::PinScope<'_, '_>,
    node: DomHandle,
    offset: u32,
) -> Option<DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let data = character_data_utf16_units_handle(scope, node)?;
    unsafe { &mut *host_ptr }.split_text(scope, host_ptr, node, offset as usize, &data)
}

pub(in crate::context_bootstrap::range_algorithms) fn range_slice_utf16_string(
    value: &str,
    start: usize,
    end: usize,
) -> String {
    utf16_slice_lossy(value, start, end)
}

pub(in crate::context_bootstrap::range_algorithms) fn parent_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.dom_host().parent_node(handle)
}

pub(in crate::context_bootstrap::range_algorithms) fn node_handle_for_tree_op<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    range_tree_op_node_handle(scope, host_ptr, node)
}

pub(in crate::context_bootstrap::range_algorithms) fn node_handle_for_range_insert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document_handle: Option<DomHandle>,
    node: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let node = moli_webapi_declare::web_api_object_target(scope, node)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    crate::native_bridge::node_or_foreign_arg_handle_allow_detached(
        scope,
        host_ptr,
        document_handle,
        node.into(),
    )
}

pub(in crate::context_bootstrap::range_algorithms) fn node_type_for_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<NodeType> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }
        .dom_host()
        .node(handle)
        .map(|node| node.node_type())
}

pub(in crate::context_bootstrap::range_algorithms) fn range_node_length_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<u32> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &*host_ptr };
    let node = runtime.dom_host().node(handle)?;
    match node.node_type() {
        NodeType::Text
        | NodeType::CDataSection
        | NodeType::ProcessingInstruction
        | NodeType::Comment => runtime
            .character_data_utf16_units(handle)
            .map(|units| units.len() as u32),
        _ => Some(runtime.dom_host().child_handles(handle).count() as u32),
    }
}

pub(in crate::context_bootstrap::range_algorithms) fn range_inserted_node_length_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<u32> {
    let node_type = node_type_for_handle(scope, handle)?;
    if node_type == NodeType::DocumentFragment {
        range_node_length_handle(scope, handle)
    } else {
        Some(1)
    }
}

pub(in crate::context_bootstrap::range_algorithms) fn prospective_child_index_after_removal_handle(
    scope: &mut v8::PinScope<'_, '_>,
    parent: DomHandle,
    reference_child: Option<DomHandle>,
    removed_child: DomHandle,
) -> Option<u32> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &*host_ptr };
    runtime.dom_host().node(parent)?;

    let mut index = 0u32;
    for child in runtime.dom_host().child_handles(parent) {
        if child == removed_child {
            continue;
        }
        if Some(child) == reference_child {
            return Some(index);
        }
        index = index.saturating_add(1);
    }

    reference_child.is_none().then_some(index)
}

pub(in crate::context_bootstrap::range_algorithms) fn child_handles_between_offsets(
    scope: &mut v8::PinScope<'_, '_>,
    container: DomHandle,
    start_offset: u32,
    end_offset: u32,
) -> Option<Vec<DomHandle>> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &*host_ptr };
    runtime.dom_host().node(container)?;
    let children = runtime
        .dom_host()
        .child_handles(container)
        .collect::<Vec<_>>();
    let start_offset = (start_offset as usize).min(children.len());
    let end_offset = (end_offset as usize).min(children.len()).max(start_offset);
    Some(children[start_offset..end_offset].to_vec())
}

pub(in crate::context_bootstrap::range_algorithms) fn child_handle_at_offset_optional(
    scope: &mut v8::PinScope<'_, '_>,
    parent: DomHandle,
    offset: u32,
) -> Option<Option<DomHandle>> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &*host_ptr };
    runtime.dom_host().node(parent)?;
    Some(runtime.dom_host().nth_child(parent, offset as usize))
}

pub(in crate::context_bootstrap::range_algorithms) fn child_index_handle(
    scope: &mut v8::PinScope<'_, '_>,
    parent: DomHandle,
    child: DomHandle,
) -> Option<u32> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }
        .dom_host()
        .child_index(parent, child)
        .and_then(|index| u32::try_from(index).ok())
}

pub(in crate::context_bootstrap::range_algorithms) fn next_sibling_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.dom_host().next_sibling(handle)
}

pub(in crate::context_bootstrap::range_algorithms) fn previous_sibling_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.prev_sibling())
}

pub(in crate::context_bootstrap::range_algorithms) fn node_contains_handle(
    scope: &mut v8::PinScope<'_, '_>,
    ancestor: DomHandle,
    node: DomHandle,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let dom = unsafe { &*host_ptr }.dom_host();
    let mut current = Some(node);
    while let Some(candidate) = current {
        if candidate == ancestor {
            return true;
        }
        current = dom.parent_node(candidate);
    }
    false
}

pub(in crate::context_bootstrap::range_algorithms) fn character_data_utf16_units_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
) -> Option<Vec<u16>> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host_ptr }.character_data_utf16_units(handle)
}

pub(in crate::context_bootstrap::range_algorithms) fn set_character_data_utf16_units_handle(
    scope: &mut v8::PinScope<'_, '_>,
    handle: DomHandle,
    units: &[u16],
) -> Option<bool> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    Some(unsafe { &mut *host_ptr }.set_character_data_utf16_units(scope, host_ptr, handle, units))
}

pub(in crate::context_bootstrap::range_algorithms) fn append_child_internal_handle(
    scope: &mut v8::PinScope<'_, '_>,
    parent: DomHandle,
    child: DomHandle,
) -> Option<()> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    append_child_internal_handle_with_host(scope, host_ptr, parent, child)
}

pub(in crate::context_bootstrap::range_algorithms) fn range_insert_move_internal_handle(
    scope: &mut v8::PinScope<'_, '_>,
    parent: DomHandle,
    child: DomHandle,
    reference_child: Option<DomHandle>,
) -> Option<()> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    crate::custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        let runtime = unsafe { &mut *host_ptr };
        runtime
            .insert_before_appending_to_current_reaction_queue(
                scope,
                host_ptr,
                parent,
                child,
                reference_child,
            )
            .then_some(())
    })
}

pub(in crate::context_bootstrap::range_algorithms) fn validate_pre_insert_internal_handle(
    scope: &mut v8::PinScope<'_, '_>,
    parent: DomHandle,
    child: DomHandle,
    reference_child: Option<DomHandle>,
    skipped: &[DomHandle],
) -> Option<()> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    crate::native_bridge::validate_pre_insert_handles(
        scope,
        unsafe { &*host_ptr },
        parent,
        child,
        reference_child,
        skipped,
    )
    .then_some(())
}

pub(in crate::context_bootstrap::range_algorithms) fn remove_child_internal_handle(
    scope: &mut v8::PinScope<'_, '_>,
    parent: DomHandle,
    child: DomHandle,
) -> Option<()> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    remove_child_internal_handle_with_host(scope, host_ptr, parent, child)
}

fn range_tree_op_node_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    node: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let node = moli_webapi_declare::web_api_object_target(scope, node)?;
    native_bridge::current_or_live_delegate_node_arg_handle(scope, host_ptr, node.into())
}

fn clone_node_internal_handle_with_host(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    deep: bool,
) -> Option<DomHandle> {
    unsafe { &mut *host_ptr }.clone_node(scope, host_ptr, handle, deep)
}

fn append_child_internal_handle_with_host(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    parent: DomHandle,
    child: DomHandle,
) -> Option<()> {
    unsafe { &mut *host_ptr }
        .append_child(scope, host_ptr, parent, child)
        .then_some(())
}

fn remove_child_internal_handle_with_host(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    parent: DomHandle,
    child: DomHandle,
) -> Option<()> {
    unsafe { &mut *host_ptr }
        .remove_child(scope, host_ptr, parent, child)
        .then_some(())
}
