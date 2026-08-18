use super::*;
use crate::{
    custom_elements,
    dom::native::DomHost,
    dom_parser::DOM_PARSER_FOREIGN_NODE_SLOT,
    util::{
        context_host_ptr_from_global_bridge, get_private_object, get_private_value,
        set_private_value,
    },
};

pub(in crate::native_bridge::document) fn detached_is_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    detached_state_object(scope, node).is_some()
}

pub(in crate::native_bridge::document) fn with_detached_tree_reaction_scope<'scope, 'pin, R>(
    scope: &mut v8::PinScope<'scope, 'pin>,
    op: impl FnOnce(&mut v8::PinScope<'scope, 'pin>) -> R,
) -> R {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return op(scope);
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, op)
}

pub(in crate::native_bridge::document) fn detached_detach_from_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    detached_remove_from_parent(scope, node);
}

pub(in crate::native_bridge::document) fn detached_detach_from_parent_appending_to_current_reaction_queue<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    let _ = detached_detach_from_native_parent_appending_to_current_reaction_queue(scope, node);
}

pub(in crate::native_bridge::document) fn detached_remove_from_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    let _ = detached_detach_from_native_parent(scope, node);
}

pub(in crate::native_bridge::document) fn detached_detach_for_insert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    detached_detach_from_native_parent(scope, node).unwrap_or(false)
}

pub(in crate::native_bridge::document) fn detached_detach_for_insert_appending_to_current_reaction_queue<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    detached_detach_from_native_parent_appending_to_current_reaction_queue(scope, node)
        .unwrap_or(false)
}

fn detached_detach_from_native_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    detached_detach_from_native_parent_with_current_queue_policy(scope, node, false)
}

fn detached_detach_from_native_parent_appending_to_current_reaction_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    detached_detach_from_native_parent_with_current_queue_policy(scope, node, true)
}

fn detached_detach_from_native_parent_with_current_queue_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    append_to_current_reaction_queue: bool,
) -> Option<bool> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let child_handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let parent_handle = unsafe { &*runtime_ptr }
        .dom_host()
        .dom()
        .parent_node(child_handle);
    let Some(parent_handle) = parent_handle else {
        return Some(false);
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let removed = if append_to_current_reaction_queue {
        runtime.remove_detached_native_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            parent_handle,
            child_handle,
        )
    } else {
        runtime.remove_detached_native_child(scope, runtime_ptr, parent_handle, child_handle)
    };
    if let Some(parent) = detached_native_object_for_handle(scope, runtime_ptr, parent_handle) {
        detached_record_tree_mutation(scope, parent);
    }
    Some(removed)
}

pub(in crate::native_bridge) fn detached_set_owner_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    owner_document: v8::Local<'s, v8::Object>,
) {
    detached_set_owner_document_inner(
        scope,
        node,
        owner_document,
        DetachedOwnerDocumentMode::DispatchAdoption,
    );
}

pub(in crate::native_bridge::document) fn detached_set_owner_document_appending_to_current_reaction_queue<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    owner_document: v8::Local<'s, v8::Object>,
) {
    detached_set_owner_document_inner(
        scope,
        node,
        owner_document,
        DetachedOwnerDocumentMode::AppendAdoptionToCurrentReactionQueue,
    );
}

#[derive(Clone, Copy)]
enum DetachedOwnerDocumentMode {
    DispatchAdoption,
    AppendAdoptionToCurrentReactionQueue,
}

fn detached_set_owner_document_inner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    owner_document: v8::Local<'s, v8::Object>,
    mode: DetachedOwnerDocumentMode,
) {
    let Some(state) = detached_state_object(scope, node) else {
        return;
    };
    if object_string_property(scope, state, "kind").is_some_and(|kind| kind == "document") {
        return;
    }
    if sync_detached_native_owner_document(scope, node, owner_document, mode) {
        return;
    }
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );
    let children = detached_child_node_objects(scope, node);
    for child in children {
        detached_set_owner_document_inner(scope, child, owner_document, mode);
    }
}

fn sync_detached_native_owner_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    owner_document: v8::Local<'s, v8::Object>,
    mode: DetachedOwnerDocumentMode,
) -> bool {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, node) else {
        return false;
    };
    let Some(document_handle) =
        detached_native_handle_for_runtime(scope, runtime_ptr, owner_document)
    else {
        return false;
    };
    if handle == document_handle {
        return true;
    }
    let adopted = match mode {
        DetachedOwnerDocumentMode::DispatchAdoption => unsafe { &mut *runtime_ptr }
            .adopt_native_node_collecting_adoption_reactions(
                scope,
                runtime_ptr,
                document_handle,
                handle,
            ),
        DetachedOwnerDocumentMode::AppendAdoptionToCurrentReactionQueue => {
            unsafe { &mut *runtime_ptr }.adopt_native_node_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                document_handle,
                handle,
            )
        }
    };
    adopted.is_some()
}

pub(in crate::native_bridge::document) fn detached_replace_children_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    children: v8::Local<'s, v8::Array>,
) {
    detached_sync_children_projection(scope, target, children, true);
}

pub(in crate::native_bridge::document) fn detached_update_existing_children_projection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    children: v8::Local<'s, v8::Array>,
) {
    detached_sync_children_projection(scope, target, children, false);
}

pub(in crate::native_bridge::document) fn detached_set_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    parent: v8::Local<'s, v8::Object>,
) {
    let Some(state) = detached_state_object(scope, node) else {
        return;
    };
    let _ = state.set(scope, v8str(scope, "parent").into(), parent.into());
}

fn detached_sync_children_projection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    children: v8::Local<'s, v8::Array>,
    create_if_missing: bool,
) {
    let Some(state) = detached_state_object(scope, target) else {
        return;
    };
    let Some(existing) = object_property_as_object(scope, state, "children") else {
        if create_if_missing {
            let _ = state.set(scope, v8str(scope, "children").into(), children.into());
        }
        return;
    };
    if existing
        .get(scope, v8str(scope, "item").into())
        .is_none_or(|value| value.is_null_or_undefined())
    {
        install_object_array_item_method(scope, existing);
    }
    let next_values = indexed_object_values(scope, children.into())
        .into_iter()
        .map(|child| v8::Global::new(scope, child))
        .collect::<Vec<_>>();
    let _ = existing.set(
        scope,
        v8str(scope, "length").into(),
        v8::Integer::new(scope, 0).into(),
    );
    for (index, child) in next_values.iter().enumerate() {
        let child = v8::Local::new(scope, child);
        let _ = existing.set_index(scope, index as u32, child.into());
    }
    let _ = existing.set(
        scope,
        v8str(scope, "length").into(),
        v8::Integer::new(scope, next_values.len() as i32).into(),
    );
}

pub(crate) fn detached_native_handle_for_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    node: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let value = get_private_value(scope, node, DETACHED_NATIVE_HANDLE_SLOT)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (index, lossless) = big.u64_value();
    if !lossless {
        return None;
    }
    let handle = DomHandle::new(index as usize);
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .is_some()
        .then_some(handle)
}

pub(in crate::native_bridge::document) fn detached_native_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    detached_native_handle_for_runtime(scope, runtime_ptr, node)
}

pub(in crate::native_bridge::document) fn detached_has_native_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    detached_native_handle(scope, node).is_some()
}

pub(in crate::native_bridge::document) fn read_detached_native_node_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<i32> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .map(|node| node.node_type() as i32)
}

pub(in crate::native_bridge::document) fn read_detached_native_node_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .map(|node| node.node_name())
}

pub(in crate::native_bridge::document) fn read_detached_native_doctype_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_document_type())
        .map(|doctype| doctype.name().to_owned())
}

pub(in crate::native_bridge::document) fn read_detached_native_doctype_public_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_document_type())
        .map(|doctype| doctype.public_id().to_owned())
}

pub(in crate::native_bridge::document) fn read_detached_native_doctype_system_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_document_type())
        .map(|doctype| doctype.system_id().to_owned())
}

pub(in crate::native_bridge::document) fn read_detached_native_processing_instruction_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.data().as_processing_instruction())
        .map(|pi| pi.target().to_owned())
}

pub(in crate::native_bridge::document) fn detached_node_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<i32> {
    read_detached_native_node_type(scope, node)
        .or_else(|| detached_state_node_type(scope, node))
        .or_else(|| object_node_type(scope, node))
}

pub(in crate::native_bridge::document) fn detached_node_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    detached_state_string(scope, node, "nodeName")
        .or_else(|| read_detached_native_node_name(scope, node))
}

pub(in crate::native_bridge) fn detached_doctype_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    read_detached_native_doctype_name(scope, node)
        .or_else(|| detached_state_string(scope, node, "name"))
}

pub(in crate::native_bridge) fn detached_doctype_public_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    read_detached_native_doctype_public_id(scope, node)
        .or_else(|| detached_state_string(scope, node, "publicId"))
}

pub(in crate::native_bridge) fn detached_doctype_system_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    read_detached_native_doctype_system_id(scope, node)
        .or_else(|| detached_state_string(scope, node, "systemId"))
}

pub(in crate::native_bridge) fn detached_processing_instruction_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    read_detached_native_processing_instruction_target(scope, node)
        .or_else(|| detached_state_string(scope, node, "target"))
}

fn detached_state_node_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<i32> {
    let state = detached_state_object(scope, node)?;
    state
        .get(scope, v8str(scope, "nodeType").into())?
        .int32_value(scope)
}

pub(crate) fn detached_native_object_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    unsafe { &*runtime_ptr }.dom_host().node(handle)?;
    let wrapper = {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, handle)
    }?;
    let object = if let Some(object) =
        get_private_object(scope, wrapper, DOM_PARSER_FOREIGN_NODE_SLOT)
    {
        object
    } else {
        let owner_document_handle = unsafe { &*runtime_ptr }
            .dom_host()
            .owner_document_handle(handle);
        owner_document_handle
            .and_then(|owner_document_handle| {
                paired_detached_native_object_for_handle(scope, runtime_ptr, owner_document_handle)
            })
            .and_then(|owner_document| {
                materialize_attached_native_node_as_detached(scope, owner_document, wrapper)
            })
            .unwrap_or(wrapper)
    };
    let value = v8::BigInt::new_from_u64(scope, handle.index() as u64);
    set_private_value(scope, object, DETACHED_NATIVE_HANDLE_SLOT, value.into());
    Some(object)
}

pub(crate) fn paired_detached_native_object_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    unsafe { &*runtime_ptr }.dom_host().node(handle)?;
    let wrapper = {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime
            .native_bridge_mut()
            .cached_handle_wrapper(scope, handle)
    }?;
    get_private_object(scope, wrapper, DOM_PARSER_FOREIGN_NODE_SLOT)
}

fn detached_native_related_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    related: impl FnOnce(&DomHost, DomHandle) -> Option<DomHandle>,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let related_handle = {
        let dom_host = unsafe { &*runtime_ptr }.dom_host();
        related(dom_host, handle)
    };
    Some(
        related_handle
            .and_then(|handle| detached_native_object_for_handle(scope, runtime_ptr, handle)),
    )
}

fn detached_native_owner_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let owner_document = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)?
        .owner_document();
    Some(
        owner_document
            .and_then(|handle| detached_native_object_for_handle(scope, runtime_ptr, handle)),
    )
}

fn detached_native_node_is_element(
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<bool> {
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .map(|node| node.is_element())
}

pub(in crate::native_bridge::document) fn detached_native_child_node_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let child_handles = unsafe { &*runtime_ptr }
        .dom_host()
        .child_handles(handle)
        .collect::<Vec<_>>();
    detached_native_child_node_objects_for_handles(scope, runtime_ptr, child_handles)
}

pub(in crate::native_bridge::document) fn detached_child_node_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    if detached_has_native_handle(scope, node) {
        return detached_native_child_node_objects(scope, node).unwrap_or_default();
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        return object_child_nodes(scope, delegate);
    }
    if detached_state_object(scope, node).is_none() {
        return Vec::new();
    }
    detached_state_children(scope, node)
}

pub(in crate::native_bridge::document) fn detached_native_mutation_child_node_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    detached_native_child_node_objects(scope, node)
}

fn detached_native_child_node_objects_for_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    child_handles: Vec<DomHandle>,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    let mut children = Vec::with_capacity(child_handles.len());
    for child in child_handles {
        children.push(detached_native_object_for_handle(
            scope,
            runtime_ptr,
            child,
        )?);
    }
    Some(children)
}

pub(in crate::native_bridge::document) fn detached_native_parent_is<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    parent: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let parent_handle = detached_native_handle_for_runtime(scope, runtime_ptr, parent)?;
    Some(
        unsafe { &*runtime_ptr }
            .dom_host()
            .dom()
            .parent_node(handle)
            == Some(parent_handle),
    )
}

fn detached_native_element_child_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    Some(
        detached_native_child_node_objects(scope, node)?
            .into_iter()
            .filter(|child| detached_node_type(scope, *child) == Some(1))
            .collect(),
    )
}

fn detached_native_contains<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    other: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let other_handle = detached_native_handle_for_runtime(scope, runtime_ptr, other)?;
    Some(
        handle == other_handle
            || unsafe { &*runtime_ptr }
                .dom_host()
                .dom()
                .contains(handle, other_handle),
    )
}

fn detached_native_is_connected_to_tree_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    let dom = dom_host.dom();
    let mut current = Some(handle);
    while let Some(candidate) = current {
        let native_node = dom.node(candidate)?;
        if native_node.is_document() {
            return Some(true);
        }
        current = native_node
            .parent_node()
            .or_else(|| dom_host.shadow_root_host(candidate));
    }
    Some(false)
}

pub(in crate::native_bridge) fn define_detached_native_handle(
    scope: &mut v8::PinScope<'_, '_>,
    node: v8::Local<'_, v8::Object>,
    handle: DomHandle,
) {
    let value = v8::BigInt::new_from_u64(scope, handle.index() as u64);
    set_private_value(scope, node, DETACHED_NATIVE_HANDLE_SLOT, value.into());
    pair_detached_native_handle_with_wrapper(scope, node, handle);
}

pub(in crate::native_bridge::document) fn sync_detached_native_insert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
    reference_child: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    sync_detached_native_insert_with_current_queue_policy(
        scope,
        parent,
        child,
        reference_child,
        false,
    )
}

pub(in crate::native_bridge::document) fn sync_detached_native_insert_appending_to_current_reaction_queue<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
    reference_child: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    sync_detached_native_insert_with_current_queue_policy(
        scope,
        parent,
        child,
        reference_child,
        true,
    )
}

fn sync_detached_native_insert_with_current_queue_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
    reference_child: Option<v8::Local<'s, v8::Object>>,
    append_to_current_reaction_queue: bool,
) -> bool {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(parent_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, parent) else {
        return false;
    };
    let Some(child_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, child) else {
        return false;
    };
    let reference_handle = reference_child.and_then(|reference_child| {
        detached_native_handle_for_runtime(scope, runtime_ptr, reference_child)
    });
    let runtime = unsafe { &mut *runtime_ptr };
    if append_to_current_reaction_queue {
        runtime.insert_detached_native_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            parent_handle,
            child_handle,
            reference_handle,
        )
    } else {
        runtime.insert_detached_native_child(
            scope,
            runtime_ptr,
            parent_handle,
            child_handle,
            reference_handle,
        )
    }
}

pub(in crate::native_bridge::document) fn sync_detached_native_set_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, element) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.set_attribute(scope, runtime_ptr, handle, name, value);
}

pub(in crate::native_bridge::document) fn sync_detached_native_set_attribute_ns<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
    value: &str,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, element) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.set_attribute_ns(
        scope,
        runtime_ptr,
        handle,
        namespace,
        prefix,
        local_name,
        local_name,
        value,
    );
}

pub(crate) fn write_detached_native_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> bool {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, element) else {
        return false;
    };
    if unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())
        .is_none()
    {
        return false;
    }
    clear_detached_iframe_context_before_navigation_attribute_change(
        scope,
        runtime_ptr,
        element,
        handle,
        name,
        Some(value),
    );
    let changed =
        unsafe { &mut *runtime_ptr }.set_attribute(scope, runtime_ptr, handle, name, value);
    if changed {
        detached_record_tree_mutation(scope, element);
    }
    true
}

pub(crate) fn write_detached_native_attribute_appending_to_current_reaction_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> bool {
    let Some((runtime_ptr, handle)) = detached_native_element_runtime_and_handle(scope, element)
    else {
        return false;
    };
    clear_detached_iframe_context_before_navigation_attribute_change(
        scope,
        runtime_ptr,
        element,
        handle,
        name,
        Some(value),
    );
    let changed =
        crate::native_bridge::element::set_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            name,
            value,
        );
    if changed {
        detached_record_tree_mutation(scope, element);
    }
    true
}

pub(crate) fn write_detached_native_attribute_ns_appending_to_current_reaction_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    prefix: Option<&str>,
    qualified_name: &str,
    local_name: &str,
    value: &str,
) -> bool {
    let Some((runtime_ptr, handle)) = detached_native_element_runtime_and_handle(scope, element)
    else {
        return false;
    };
    let changed = crate::native_bridge::element::set_live_element_attribute_ns_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        namespace,
        prefix,
        local_name,
        qualified_name,
        value,
    );
    if changed {
        detached_record_tree_mutation(scope, element);
    }
    true
}

pub(crate) fn remove_detached_native_attribute_appending_to_current_reaction_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    let Some((runtime_ptr, handle)) = detached_native_element_runtime_and_handle(scope, element)
    else {
        return false;
    };
    clear_detached_iframe_context_before_navigation_attribute_change(
        scope,
        runtime_ptr,
        element,
        handle,
        name,
        None,
    );
    let removed = crate::native_bridge::element::remove_live_element_attribute_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        name,
    );
    if removed {
        detached_record_tree_mutation(scope, element);
    }
    true
}

pub(crate) fn remove_detached_native_attribute_ns_appending_to_current_reaction_queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    local_name: &str,
) -> bool {
    let Some((runtime_ptr, handle)) = detached_native_element_runtime_and_handle(scope, element)
    else {
        return false;
    };
    if namespace.is_none() {
        clear_detached_iframe_context_before_navigation_attribute_change(
            scope,
            runtime_ptr,
            element,
            handle,
            local_name,
            None,
        );
    }
    let removed = crate::native_bridge::element::remove_live_element_attribute_ns_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        namespace,
        local_name,
    );
    if removed {
        detached_record_tree_mutation(scope, element);
    }
    true
}

fn clear_detached_iframe_context_before_navigation_attribute_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    element: v8::Local<'s, v8::Object>,
    handle: DomHandle,
    name: &str,
    next_value: Option<&str>,
) {
    if !name.eq_ignore_ascii_case("src") && !name.eq_ignore_ascii_case("srcdoc") {
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "iframe") {
        return;
    }
    let current_value = runtime.dom_host().get_attribute(handle, name);
    let changes = match next_value {
        Some(next_value) => current_value.as_deref() != Some(next_value),
        None => current_value.is_some(),
    };
    if changes {
        crate::native_bridge::document::clear_detached_iframe_cached_context(scope, element);
    }
}

pub(in crate::native_bridge::document) fn detached_native_element_runtime_and_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.as_element())?;
    Some((runtime_ptr, handle))
}

pub(crate) fn with_detached_native_element_reaction_scope<'scope, 'pin, R>(
    scope: &mut v8::PinScope<'scope, 'pin>,
    element: v8::Local<'scope, v8::Object>,
    op: impl FnOnce(&mut v8::PinScope<'scope, 'pin>) -> R,
) -> Option<R> {
    let (runtime_ptr, _) = detached_native_element_runtime_and_handle(scope, element)?;
    Some(custom_elements::with_custom_element_reaction_scope(
        scope,
        runtime_ptr,
        op,
    ))
}

pub(crate) fn read_detached_native_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<String> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    dom_host.get_attribute(handle, name)
}

pub(crate) fn read_detached_native_attribute_ns<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    local_name: &str,
) -> Option<String> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    dom_host.get_attribute_ns(handle, namespace, local_name)
}

pub(crate) fn read_detached_native_attribute_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<Vec<String>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    dom_host.dom().get_attribute_names(handle)
}

pub(crate) struct DetachedNativeAttributeSnapshot {
    pub(crate) name: String,
    pub(in crate::native_bridge::document) value: String,
    pub(in crate::native_bridge::document) namespace_uri: Option<String>,
    pub(in crate::native_bridge::document) prefix: Option<String>,
    pub(in crate::native_bridge::document) local_name: String,
}

pub(crate) fn read_detached_native_attribute_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<Vec<DetachedNativeAttributeSnapshot>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    let element = dom_host.node(handle).and_then(|node| node.as_element())?;
    Some(
        element
            .attributes()
            .iter()
            .map(|attribute| DetachedNativeAttributeSnapshot {
                name: attribute.name(),
                value: attribute.value().to_owned(),
                namespace_uri: (!attribute.namespace().is_empty())
                    .then(|| attribute.namespace().to_owned()),
                prefix: attribute.prefix().map(str::to_owned),
                local_name: attribute.local_name().to_owned(),
            })
            .collect(),
    )
}

pub(crate) fn read_detached_native_has_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<bool> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    dom_host
        .node(handle)
        .and_then(|node| node.as_element())
        .map(|_| dom_host.dom().has_attribute(handle, name))
}

pub(crate) fn read_detached_native_has_attribute_ns<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    local_name: &str,
) -> Option<bool> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    dom_host
        .node(handle)
        .and_then(|node| node.as_element())
        .map(|_| {
            dom_host
                .dom()
                .has_attribute_ns(handle, namespace, local_name)
        })
}

pub(in crate::native_bridge::document) fn read_detached_native_text_content<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    let dom_host = unsafe { &*runtime_ptr }.dom_host();
    dom_host.text_content(handle)
}

pub(in crate::native_bridge::document) fn write_detached_native_text_content<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    value: &str,
) -> Option<bool> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    Some(unsafe { &mut *runtime_ptr }.set_text_content(scope, runtime_ptr, handle, value))
}

pub(in crate::native_bridge::document) fn write_detached_native_text_content_appending_to_current_reaction_queue<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    value: &str,
) -> Option<bool> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, node)?;
    Some(
        unsafe { &mut *runtime_ptr }.set_text_content_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            value,
        ),
    )
}

fn pair_detached_native_handle_with_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    node: v8::Local<'_, v8::Object>,
    handle: DomHandle,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    if unsafe { &*runtime_ptr }.dom_host().node(handle).is_none() {
        return;
    }
    let wrapper = {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, handle)
    };
    let Some(wrapper) = wrapper else {
        return;
    };
    set_private_value(scope, wrapper, DOM_PARSER_FOREIGN_NODE_SLOT, node.into());
}

pub(in crate::native_bridge::document) fn detached_tree_root_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if detached_state_kind(scope, node).as_deref() == Some("document") {
        return Some(node);
    }
    detached_owner_document_object(scope, node).or_else(|| {
        let mut current = detached_parent_node_object(scope, node);
        while let Some(candidate) = current {
            if detached_state_kind(scope, candidate).as_deref() == Some("document") {
                return Some(candidate);
            }
            current = detached_parent_node_object(scope, candidate);
        }
        None
    })
}

pub(in crate::native_bridge::document) fn detached_tree_query_version<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let root = detached_tree_root_object(scope, node)?;
    let state = detached_state_object(scope, root)?;
    let Some(value) = state.get(scope, v8str(scope, "queryVersion").into()) else {
        return Some(0);
    };
    if value.is_null_or_undefined() {
        return Some(0);
    }
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (version, _lossless) = big.u64_value();
        return Some(version);
    }
    value.uint32_value(scope).map(u64::from)
}

pub(in crate::native_bridge::document) fn detached_record_tree_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    let Some(root) = detached_tree_root_object(scope, node) else {
        return;
    };
    let Some(state) = detached_state_object(scope, root) else {
        return;
    };
    let next = detached_tree_query_version(scope, root)
        .unwrap_or(0)
        .saturating_add(1);
    let _ = state.set(
        scope,
        v8str(scope, "queryVersion").into(),
        v8::BigInt::new_from_u64(scope, next).into(),
    );
}

pub(in crate::native_bridge::document) fn detached_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, node, DETACHED_STATE_SLOT)
}

pub(in crate::native_bridge::document) fn detached_live_delegate_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, node, DETACHED_LIVE_DELEGATE_SLOT)
}

pub(in crate::native_bridge::document) fn detached_state_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let state = detached_state_object(scope, node)?;
    object_string_property(scope, state, "kind")
}

pub(in crate::native_bridge::document) fn detached_state_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<String> {
    let state = detached_state_object(scope, node)?;
    object_string_property(scope, state, key)
}

pub(in crate::native_bridge) fn detached_parent_node_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if detached_has_native_handle(scope, node) {
        if let Some(parent) = detached_native_related_object(scope, node, |dom_host, handle| {
            dom_host.dom().parent_node(handle)
        })
        .flatten()
        {
            return Some(parent);
        }
        return None;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        return object_property_as_object(scope, delegate, "parentNode");
    }
    if let Some(state) = detached_state_object(scope, node) {
        object_property_as_object(scope, state, "parent")
    } else {
        None
    }
}

pub(in crate::native_bridge::document) fn detached_parent_element_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, node)
    {
        let parent_handle = unsafe { &*runtime_ptr }
            .dom_host()
            .dom()
            .parent_node(handle);
        if let Some(parent_handle) = parent_handle {
            return detached_native_node_is_element(runtime_ptr, parent_handle)
                .filter(|is_element| *is_element)
                .and_then(|_| {
                    detached_native_object_for_handle(scope, runtime_ptr, parent_handle)
                });
        }
        return None;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        return object_property_as_object(scope, delegate, "parentElement");
    }
    let parent = detached_parent_node_object(scope, node)?;
    (detached_node_type(scope, parent) == Some(1)).then_some(parent)
}

pub(in crate::native_bridge::document) fn detached_owner_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(owner_document) = detached_native_owner_document_object(scope, node) {
        return owner_document;
    }
    if detached_state_kind(scope, node).as_deref() != Some("document")
        && let Some(owner_document) = detached_state_object(scope, node)
            .and_then(|state| object_property_as_object(scope, state, "ownerDocument"))
        && detached_state_kind(scope, owner_document).as_deref() == Some("document")
    {
        return Some(owner_document);
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        return object_property_as_object(scope, delegate, "ownerDocument");
    }
    if detached_state_kind(scope, node).as_deref() == Some("document") {
        return None;
    }
    detached_state_object(scope, node)
        .and_then(|state| object_property_as_object(scope, state, "ownerDocument"))
}

pub(in crate::native_bridge::document) fn detached_sibling_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    delta: isize,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, node)
    {
        let (has_native_parent, sibling_handle) = {
            let dom_host = unsafe { &*runtime_ptr }.dom_host();
            let dom = dom_host.dom();
            let sibling = if delta < 0 {
                dom.previous_sibling(handle)
            } else {
                dom.next_sibling(handle)
            };
            (dom.parent_node(handle).is_some(), sibling)
        };
        if let Some(sibling_handle) = sibling_handle {
            return detached_native_object_for_handle(scope, runtime_ptr, sibling_handle);
        }
        if has_native_parent {
            return None;
        }
        return None;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        let key = if delta < 0 {
            "previousSibling"
        } else {
            "nextSibling"
        };
        return object_property_as_object(scope, delegate, key);
    }
    let parent = detached_parent_node_object(scope, node)?;
    let siblings = detached_state_children(scope, parent);
    let index = siblings
        .iter()
        .position(|candidate| candidate.strict_equals(node.into()))?;
    let target = index as isize + delta;
    if target < 0 {
        return None;
    }
    siblings.get(target as usize).copied()
}

pub(crate) fn detached_is_connected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    if detached_native_is_connected_to_tree_root(scope, node) == Some(true) {
        return true;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node)
        && object_property_value(scope, delegate, "isConnected")
            .is_some_and(|value| value.boolean_value(scope))
    {
        return true;
    }
    let mut current = Some(node);
    while let Some(candidate) = current {
        if detached_state_kind(scope, candidate).as_deref() == Some("document") {
            return true;
        }
        current = detached_parent_node_object(scope, candidate).or_else(|| {
            (detached_state_kind(scope, candidate).as_deref() == Some("shadowRoot"))
                .then(|| {
                    detached_state_object(scope, candidate)
                        .and_then(|state| object_property_as_object(scope, state, "host"))
                })
                .flatten()
        });
    }
    false
}

pub(in crate::native_bridge::document) fn detached_contains<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    other: v8::Local<'s, v8::Object>,
) -> bool {
    if node.strict_equals(other.into()) {
        return true;
    }
    if let Some(contains) = detached_native_contains(scope, node, other) {
        return contains;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        let candidate = detached_live_delegate_object(scope, other).unwrap_or(other);
        return call_object_method(scope, delegate, "contains", &[candidate.into()])
            .is_some_and(|value| value.boolean_value(scope));
    }
    let mut current = detached_parent_node_object(scope, other);
    while let Some(candidate) = current {
        if candidate.strict_equals(node.into()) {
            return true;
        }
        current = detached_parent_node_object(scope, candidate);
    }
    false
}

pub(in crate::native_bridge::document) fn detached_element_children_objects<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    if let Some(children) = detached_native_element_child_objects(scope, node) {
        return children;
    }
    if let Some(delegate) = detached_live_delegate_object(scope, node) {
        return object_property_as_object(scope, delegate, "children")
            .map(|children| indexed_object_values(scope, children))
            .unwrap_or_default();
    }
    detached_state_children(scope, node)
        .into_iter()
        .filter(|child| detached_node_type(scope, *child) == Some(1))
        .collect()
}

fn detached_state_children<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    detached_state_object(scope, node)
        .and_then(|state| object_property_as_object(scope, state, "children"))
        .map(|children| indexed_object_values(scope, children))
        .unwrap_or_default()
}

pub(in crate::native_bridge::document) fn detached_element_sibling_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    delta: isize,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut current = detached_sibling_object(scope, node, delta);
    while let Some(candidate) = current {
        if detached_node_type(scope, candidate) == Some(1) {
            return Some(candidate);
        }
        current = detached_sibling_object(scope, candidate, delta);
    }
    None
}

pub(in crate::native_bridge::document) fn define_detached_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, object, DETACHED_STATE_SLOT, state.into());
}
