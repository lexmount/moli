use super::super::native_bridge::{PageScriptHost, wrapped_handle_value};
use super::lifecycle::{custom_element_callback_receiver, invoke_custom_element_callback};
use super::reactions::{enqueue_custom_element_reaction, enter_custom_element_reaction};
use super::{AdoptionCallbackTarget, CustomElementReaction};

pub(crate) fn enqueue_adopted_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    target: AdoptionCallbackTarget,
) -> bool {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(target.handle)
        .is_some_and(|store| store.is_upgraded_handle(target.handle))
    {
        return false;
    }
    if unsafe { &*host_ptr }
        .custom_elements_for_node_handle(target.handle)
        .and_then(|store| {
            store.lifecycle_callback_for_handle(scope, host_ptr, target.handle, "adoptedCallback")
        })
        .is_none()
    {
        return false;
    }
    enqueue_custom_element_reaction(
        scope,
        host_ptr,
        target.handle,
        CustomElementReaction::Adopted {
            old_document: target.old_document,
            new_document: target.new_document,
        },
    );
    true
}

pub(super) fn call_adopted_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    target: AdoptionCallbackTarget,
) {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(target.handle)
        .is_some_and(|store| store.is_upgraded_handle(target.handle))
    {
        return;
    }
    let Some(wrapper) = custom_element_callback_receiver(scope, host_ptr, target.handle) else {
        return;
    };
    let Some(callback) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(target.handle)
        .and_then(|store| {
            store.lifecycle_callback_for_handle(scope, host_ptr, target.handle, "adoptedCallback")
        })
    else {
        return;
    };
    let Some(old_document) = wrapped_handle_value(scope, host_ptr, target.old_document) else {
        return;
    };
    let Some(new_document) = wrapped_handle_value(scope, host_ptr, target.new_document) else {
        return;
    };
    let args = [old_document, new_document];
    let _reaction = enter_custom_element_reaction(host_ptr);
    invoke_custom_element_callback(
        scope,
        host_ptr,
        "custom element adoptedCallback",
        callback,
        wrapper.into(),
        &args,
    );
}

pub(crate) fn enqueue_adopted_callbacks(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    targets: &[AdoptionCallbackTarget],
) -> bool {
    if unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent() {
        return false;
    }
    let mut enqueued = false;
    for target in targets {
        if enqueue_adopted_callback(scope, host_ptr, *target) {
            enqueued = true;
        }
    }
    enqueued
}
