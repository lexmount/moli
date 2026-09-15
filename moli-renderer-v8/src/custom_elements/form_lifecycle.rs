use super::element_state::is_form_associated_custom_element_handle;
use super::existing_upgrade::has_pending_upgrade_reaction;
use super::reaction_types::FormStateRestoreMode;
use super::reactions::{
    CustomElementReaction, enqueue_custom_element_reaction, with_custom_element_reaction_scope,
};

use super::super::{
    document_runtime::DomHandle,
    native_bridge::{
        JsContextHost,
        element::{form_associated_form_owner, form_control_is_effectively_disabled},
    },
};

pub(crate) fn enqueue_form_association_callback_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    if has_pending_upgrade_reaction(host_ptr, handle) {
        return false;
    }
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return false;
    }
    if !is_form_associated_custom_element_handle(unsafe { &*host_ptr }, handle) {
        return false;
    }

    let current_form = form_associated_form_owner(unsafe { &*host_ptr }, handle);
    let previous_form = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.form_association_state(handle));
    if previous_form.is_none() && current_form.is_none() {
        return false;
    }
    if previous_form == Some(current_form) {
        return false;
    }

    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(handle)
        .set_form_association_state(handle, current_form);
    if unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.form_associated_callback_for_handle(scope, host_ptr, handle))
        .is_none()
    {
        return false;
    }
    enqueue_custom_element_reaction(
        scope,
        host_ptr,
        handle,
        CustomElementReaction::FormAssociated { form: current_form },
    );
    true
}

pub(super) fn enqueue_form_disabled_callback_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    if has_pending_upgrade_reaction(host_ptr, handle) {
        return false;
    }
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return false;
    }
    if !is_form_associated_custom_element_handle(unsafe { &*host_ptr }, handle) {
        return false;
    }

    let disabled = form_control_is_effectively_disabled(unsafe { &*host_ptr }, handle);
    let previous_disabled = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.form_disabled_state(handle));
    if previous_disabled.is_none() && !disabled {
        return false;
    }
    if previous_disabled == Some(disabled) {
        return false;
    }

    unsafe { &mut *host_ptr }
        .custom_elements_mut_for_node_handle(handle)
        .set_form_disabled_state(handle, disabled);
    if unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.form_disabled_callback_for_handle(scope, host_ptr, handle))
        .is_none()
    {
        return false;
    }
    enqueue_custom_element_reaction(
        scope,
        host_ptr,
        handle,
        CustomElementReaction::FormDisabled { disabled },
    );
    true
}

pub(crate) fn dispatch_form_association_callback_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        enqueue_form_association_callback_if_needed(scope, host_ptr, handle);
    });
}

pub(crate) fn dispatch_form_disabled_callback_if_needed(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        enqueue_form_disabled_callback_if_needed(scope, host_ptr, handle);
    });
}

// The session-history/autofill owner calls this once restoration is implemented.
// Keep the reaction path here so callback invocation cannot bypass CE ordering.
#[allow(dead_code)]
pub(crate) fn dispatch_form_state_restore_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    value: String,
    mode: FormStateRestoreMode,
) {
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        enqueue_custom_element_reaction(
            scope,
            host_ptr,
            handle,
            CustomElementReaction::FormStateRestore { value, mode },
        );
    });
}
