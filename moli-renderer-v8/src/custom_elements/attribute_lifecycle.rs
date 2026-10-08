use std::sync::Arc;

use moli_dom::native::DomStringValue;

use super::super::{document_runtime::DomHandle, native_bridge::JsContextHost, util::v8_string};
use super::CustomElementReaction;
use super::lifecycle::{custom_element_callback_receiver, invoke_custom_element_callback};
use super::reactions::{
    enqueue_custom_element_reaction, enter_custom_element_reaction,
    with_custom_element_reaction_scope,
};

pub(crate) fn enqueue_attribute_changed_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    namespace: Option<&str>,
    old_value: Option<Arc<DomStringValue>>,
    new_value: Option<Arc<DomStringValue>>,
) -> bool {
    if !unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.is_upgraded_handle(handle))
    {
        return false;
    }
    let observed = unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.observes_attribute_for_handle(host_ptr, handle, name));
    if !observed {
        return false;
    }
    if unsafe { &*host_ptr }
        .custom_elements_for_node_handle(handle)
        .and_then(|store| store.attribute_changed_callback_for_handle(scope, host_ptr, handle))
        .is_none()
    {
        return false;
    }
    enqueue_custom_element_reaction(
        scope,
        host_ptr,
        handle,
        CustomElementReaction::AttributeChanged {
            name: name.to_owned(),
            namespace: namespace.map(str::to_owned),
            old_value,
            new_value,
        },
    );
    true
}

pub(super) fn call_attribute_changed_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    namespace: Option<&str>,
    old_value: Option<&DomStringValue>,
    new_value: Option<&DomStringValue>,
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
        .and_then(|store| store.attribute_changed_callback_for_handle(scope, host_ptr, handle))
    else {
        return;
    };
    let Some(name_value) = v8_string(scope, name) else {
        return;
    };
    let old_value = old_value
        .and_then(|value| crate::util::v8_string_from_dom_string_value(scope, value))
        .map(Into::<v8::Local<'_, v8::Value>>::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let new_value = new_value
        .and_then(|value| crate::util::v8_string_from_dom_string_value(scope, value))
        .map(Into::<v8::Local<'_, v8::Value>>::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let namespace_value = namespace
        .and_then(|value| v8_string(scope, value))
        .map(Into::<v8::Local<'_, v8::Value>>::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let _reaction = enter_custom_element_reaction(host_ptr);
    invoke_custom_element_callback(
        scope,
        host_ptr,
        "custom element attributeChangedCallback",
        callback,
        wrapper.into(),
        &[name_value.into(), old_value, new_value, namespace_value],
    );
}

pub(crate) fn dispatch_attribute_changed_callback(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    namespace: Option<&str>,
    old_value: Option<Arc<DomStringValue>>,
    new_value: Option<Arc<DomStringValue>>,
) {
    with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        enqueue_attribute_changed_callback(
            scope, host_ptr, handle, name, namespace, old_value, new_value,
        );
    });
}
