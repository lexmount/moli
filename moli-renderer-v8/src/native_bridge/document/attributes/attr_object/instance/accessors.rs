use super::value::{
    attr_current_string_value, attr_owner_document_object, attr_owner_element_object,
    nullable_attr_state_string,
};
use super::*;
use crate::custom_elements;
use crate::native_bridge::element::{
    set_live_element_attribute_ns_utf16_units_appending_to_current_reaction_queue,
    set_live_element_attribute_utf16_units_appending_to_current_reaction_queue,
};
use crate::native_bridge::node_runtime_and_handle_from_object_or_detached;
use crate::webidl;

pub(super) fn attr_instance_node_type_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    _args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Integer::new(scope, 2).into());
}

pub(super) fn attr_instance_node_name_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    args: v8::PropertyCallbackArguments<'a>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    attr_instance_name_getter(scope, _key, args, rv);
}

pub(super) fn attr_instance_name_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = attr_state_object(scope, args.holder())
        .and_then(|state| object_string_property(scope, state, "name"))
        .unwrap_or_default();
    set_string_return_value(scope, &mut rv, &value);
}

pub(super) fn attr_instance_local_name_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = attr_state_object(scope, args.holder())
        .and_then(|state| object_string_property(scope, state, "localName"))
        .unwrap_or_default();
    set_string_return_value(scope, &mut rv, &value);
}

pub(super) fn attr_instance_prefix_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = attr_state_object(scope, args.holder())
        .and_then(|state| state.get(scope, v8str(scope, "prefix").into()));
    match value {
        Some(value) if !value.is_null_or_undefined() => rv.set(value),
        _ => rv.set_null(),
    }
}

pub(super) fn attr_instance_namespace_uri_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = attr_state_object(scope, args.holder())
        .and_then(|state| state.get(scope, v8str(scope, "namespaceURI").into()));
    match value {
        Some(value) if !value.is_null_or_undefined() => rv.set(value),
        _ => rv.set_null(),
    }
}

pub(super) fn attr_instance_owner_element_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match attr_owner_element_object(scope, args.holder()) {
        Some(owner) => rv.set(owner.into()),
        None => rv.set_null(),
    }
}

pub(super) fn attr_instance_owner_document_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match attr_owner_document_object(scope, args.holder()) {
        Some(document) => rv.set(document.into()),
        None => rv.set_null(),
    }
}

pub(super) fn attr_instance_specified_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    _args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Boolean::new(scope, true).into());
}

pub(super) fn attr_instance_base_uri_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    _args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    let value = object_property_as_object(scope, global, "document")
        .and_then(|document| object_string_property(scope, document, "baseURI"))
        .unwrap_or_default();
    set_string_return_value(scope, &mut rv, &value);
}

pub(super) fn attr_instance_value_getter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    args: v8::PropertyCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value =
        attr_current_string_value(scope, args.holder()).unwrap_or_else(|| v8::String::empty(scope));
    rv.set(value.into());
}

pub(super) fn attr_instance_value_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) {
    let nullable = key.strict_equals(v8str(scope, "nodeValue").into())
        || key.strict_equals(v8str(scope, "textContent").into());
    let value = if nullable && value.is_null_or_undefined() {
        v8::String::empty(scope).into()
    } else {
        value
    };
    let units = match webidl::convert::<webidl::DomString16>(
        scope,
        value,
        webidl::Context::member("Attr", "value"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Some(state) = attr_state_object(scope, args.holder()) else {
        return;
    };
    let Some(js_value) = crate::util::v8_string_from_utf16_units(scope, &units) else {
        return;
    };
    let _ = state.set(scope, v8str(scope, "value").into(), js_value.into());
    let Some(name) = object_string_property(scope, state, "name") else {
        return;
    };
    let Some(owner) = object_property_as_object(scope, state, "ownerElement") else {
        return;
    };
    let namespace = nullable_attr_state_string(scope, state, "namespaceURI");
    let local_name = object_string_property(scope, state, "localName")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| name.clone());
    let prefix = nullable_attr_state_string(scope, state, "prefix");
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, owner)
    {
        let value = String::from_utf16_lossy(&units);
        let normalized = unsafe { &*runtime_ptr }
            .dom_host()
            .dom()
            .normalized_attribute_name(handle, &name);
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if namespace.is_none() && normalized.as_deref() == Some(name.as_str()) {
                let _ = set_live_element_attribute_utf16_units_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    handle,
                    &name,
                    &value,
                    units,
                );
            } else {
                let _ =
                    set_live_element_attribute_ns_utf16_units_appending_to_current_reaction_queue(
                        scope,
                        runtime_ptr,
                        handle,
                        namespace.as_deref(),
                        prefix.as_deref(),
                        &local_name,
                        &name,
                        &value,
                        units,
                    );
            }
        });
        return;
    }
    // Compatibility objects without a native handle retain their JS DOMString.
    let name = v8_string(scope, &name).unwrap_or_else(|| v8::String::empty(scope));
    if namespace.is_none() {
        let _ = call_object_method(
            scope,
            owner,
            "setAttribute",
            &[name.into(), js_value.into()],
        );
    } else {
        let namespace = v8_string(scope, namespace.as_deref().unwrap())
            .unwrap_or_else(|| v8::String::empty(scope));
        let _ = call_object_method(
            scope,
            owner,
            "setAttributeNS",
            &[namespace.into(), name.into(), js_value.into()],
        );
    }
}
