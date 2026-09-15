use super::reactions::{enqueue_custom_element_reaction, with_custom_element_reaction_scope};
use super::{CustomElementReaction, has_pending_upgrade_reaction};

use super::super::{document_runtime::DomHandle, native_bridge::JsContextHost};

fn enqueue_lifecycle_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    callback_name: &str,
    reaction: CustomElementReaction,
) -> bool {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return false;
    }
    if unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| {
            store.lifecycle_callback_for_handle(scope, host_ptr, handle, callback_name)
        })
        .is_none()
    {
        return false;
    }
    enqueue_custom_element_reaction(scope, host_ptr, handle, reaction);
    true
}

pub(crate) fn enqueue_connected_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    if has_pending_upgrade_reaction(host_ptr, handle) {
        return false;
    }
    enqueue_lifecycle_callback(
        scope,
        host_ptr,
        handle,
        "connectedCallback",
        CustomElementReaction::Connected,
    )
}

pub(crate) fn enqueue_disconnected_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    if has_pending_upgrade_reaction(host_ptr, handle) {
        return false;
    }
    enqueue_lifecycle_callback(
        scope,
        host_ptr,
        handle,
        "disconnectedCallback",
        CustomElementReaction::Disconnected,
    )
}

pub(crate) fn enqueue_disconnected_callback_unless_pending(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    if unsafe { &*host_ptr }
        .custom_element_reactions()
        .pending_reactions_end_with(handle, &CustomElementReaction::Disconnected)
    {
        return false;
    }
    enqueue_disconnected_callback(scope, host_ptr, handle)
}

pub(crate) fn enqueue_connected_move_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    enqueue_lifecycle_callback(
        scope,
        host_ptr,
        handle,
        "connectedMoveCallback",
        CustomElementReaction::ConnectedMove,
    )
}

pub(crate) fn dispatch_connected_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        enqueue_connected_callback(scope, host_ptr, handle);
    });
}
