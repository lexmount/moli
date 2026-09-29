use super::super::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, element::form_associated_form_owner},
};
use super::element_state::is_form_associated_custom_element_handle;
use super::form_lifecycle::{
    enqueue_form_association_callback_if_needed, enqueue_form_disabled_callback_if_needed,
};
use super::reactions::{
    CustomElementReaction, enqueue_custom_element_reaction, with_custom_element_reaction_scope,
};
use super::traversal::collect_shadow_including_subtree_handles;

pub(crate) fn dispatch_form_reset_callbacks_for_form(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    form: DomHandle,
) {
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        enqueue_form_reset_callbacks_for_form(scope, host_ptr, form);
    });
}

pub(crate) fn enqueue_form_reset_callbacks_for_form(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    form: DomHandle,
) {
    let handles = unsafe { &*host_ptr }
        .dom_host()
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(index, _)| {
            let handle = DomHandle::new(index);
            (is_form_associated_custom_element_handle(unsafe { &*host_ptr }, handle)
                && form_associated_form_owner(unsafe { &*host_ptr }, handle) == Some(form))
            .then_some(handle)
        })
        .collect::<Vec<_>>();
    for handle in handles {
        enqueue_custom_element_reaction(scope, host_ptr, handle, CustomElementReaction::FormReset);
    }
}

pub(crate) fn enqueue_form_association_callbacks_for_all(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let host = unsafe { &*host_ptr };
    // Only upgraded elements can receive reactions. Enumerating their native
    // stores keeps ID changes independent of unrelated nodes in the DOM arena.
    let mut handles = host
        .upgraded_custom_element_handles()
        .filter(|&handle| is_form_associated_custom_element_handle(host, handle))
        .collect::<Vec<_>>();
    handles.sort_unstable_by_key(|handle| handle.index());
    handles.dedup();
    for handle in handles {
        enqueue_form_association_callback_if_needed(scope, host_ptr, handle);
    }
}

pub(crate) fn enqueue_form_disabled_callbacks_in_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) -> bool {
    let mut handles = Vec::new();
    collect_shadow_including_subtree_handles(host_ptr, root, &mut handles);
    let mut enqueued = false;
    for handle in handles {
        if enqueue_form_disabled_callback_if_needed(scope, host_ptr, handle) {
            enqueued = true;
        }
    }
    enqueued
}

pub(crate) fn dispatch_form_association_callbacks_for_all(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        enqueue_form_association_callbacks_for_all(scope, host_ptr);
    });
}

pub(crate) fn dispatch_form_disabled_callbacks_in_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) {
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        enqueue_form_disabled_callbacks_in_subtree(scope, host_ptr, root);
    });
}
