use super::lifecycle::{custom_element_callback_receiver, invoke_custom_element_callback};
use super::reactions::enter_custom_element_reaction;

use super::super::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, wrapped_handle_value},
};

pub(super) fn call_form_associated_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    form: Option<DomHandle>,
) {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return;
    }
    let Some(wrapper) = custom_element_callback_receiver(scope, host_ptr, handle) else {
        return;
    };
    let Some(callback) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.form_associated_callback_for_handle(scope, host_ptr, handle))
    else {
        return;
    };
    let form_value = form
        .and_then(|form_handle| wrapped_handle_value(scope, host_ptr, form_handle))
        .unwrap_or_else(|| v8::null(scope).into());
    let _reaction = enter_custom_element_reaction(host_ptr);
    invoke_custom_element_callback(
        scope,
        host_ptr,
        "custom element formAssociatedCallback",
        callback,
        wrapper.into(),
        &[form_value],
    );
}

pub(super) fn call_form_disabled_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    disabled: bool,
) {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return;
    }
    let Some(wrapper) = custom_element_callback_receiver(scope, host_ptr, handle) else {
        return;
    };
    let Some(callback) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.form_disabled_callback_for_handle(scope, host_ptr, handle))
    else {
        return;
    };
    let disabled = v8::Boolean::new(scope, disabled).into();
    let _reaction = enter_custom_element_reaction(host_ptr);
    invoke_custom_element_callback(
        scope,
        host_ptr,
        "custom element formDisabledCallback",
        callback,
        wrapper.into(),
        &[disabled],
    );
}

pub(super) fn call_form_reset_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return;
    }
    let Some(wrapper) = custom_element_callback_receiver(scope, host_ptr, handle) else {
        return;
    };
    let Some(callback) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.form_reset_callback_for_handle(scope, host_ptr, handle))
    else {
        return;
    };
    let _reaction = enter_custom_element_reaction(host_ptr);
    invoke_custom_element_callback(
        scope,
        host_ptr,
        "custom element formResetCallback",
        callback,
        wrapper.into(),
        &[],
    );
}

pub(super) fn call_form_state_restore_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    value: &str,
    mode: super::reaction_types::FormStateRestoreMode,
) {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return;
    }
    let Some(wrapper) = custom_element_callback_receiver(scope, host_ptr, handle) else {
        return;
    };
    let Some(callback) = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.form_state_restore_callback_for_handle(scope, host_ptr, handle))
    else {
        return;
    };
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    let Some(mode) = v8::String::new(scope, mode.as_str()) else {
        return;
    };
    let _reaction = enter_custom_element_reaction(host_ptr);
    invoke_custom_element_callback(
        scope,
        host_ptr,
        "custom element formStateRestoreCallback",
        callback,
        wrapper.into(),
        &[value.into(), mode.into()],
    );
}
