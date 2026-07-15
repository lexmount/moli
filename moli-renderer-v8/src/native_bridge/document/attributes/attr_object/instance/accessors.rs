use super::value::{attr_current_value, attr_owner_document_object, attr_owner_element_object};
use super::*;
use crate::custom_elements;
use crate::native_bridge::element::{
    TrustedAttributeSetter, set_live_element_attribute_appending_to_current_reaction_queue,
    set_live_element_attribute_ns_appending_to_current_reaction_queue,
    trusted_attribute_string_value,
};
use crate::native_bridge::node_runtime_and_handle_from_object;
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
    let value = attr_current_value(scope, args.holder());
    set_string_return_value(scope, &mut rv, &value);
}

pub(super) fn attr_instance_value_setter<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    _key: v8::Local<'a, v8::Name>,
    value: v8::Local<'a, v8::Value>,
    args: v8::PropertyCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, ()>,
) {
    let string_value = match webidl::convert::<webidl::DomString>(
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
    let attr = args.holder();
    let Some(state) = attr_state_object(scope, attr) else {
        return;
    };
    let Some(name) = object_string_property(scope, state, "name") else {
        return;
    };
    let Some(original_owner) = object_property_as_object(scope, state, "ownerElement") else {
        set_attr_state_value(scope, state, &string_value);
        return;
    };
    let namespace = attr_state_nullable_string(scope, state, "namespaceURI");
    let local_name = object_string_property(scope, state, "localName")
        .filter(|local_name| !local_name.is_empty())
        .or_else(|| name.rsplit_once(':').map(|(_, local)| local.to_owned()))
        .unwrap_or_else(|| name.clone());
    let runtime_and_handle = node_runtime_and_handle_from_object(scope, original_owner)
        .ok()
        .or_else(|| detached_native_element_runtime_and_handle(scope, original_owner));
    let Some(verified_value) = trusted_attribute_string_value(
        scope,
        runtime_and_handle,
        namespace.as_deref(),
        &local_name,
        &string_value,
        TrustedAttributeSetter::AttrValue,
    ) else {
        return;
    };

    let Some(state) = attr_state_object(scope, attr) else {
        return;
    };
    let Some(owner) = object_property_as_object(scope, state, "ownerElement") else {
        set_attr_state_value(scope, state, &verified_value);
        return;
    };
    if !owner.strict_equals(original_owner.into()) {
        return;
    }

    set_attr_state_value(scope, state, &verified_value);
    if attr_value_setter_wrote_live_native(scope, owner, state, &name, &verified_value) {
        return;
    }
    if attr_value_setter_wrote_native(scope, owner, state, &name, &verified_value) {
        return;
    }
    let namespace_value = state
        .get(scope, v8str(scope, "namespaceURI").into())
        .unwrap_or_else(|| v8::null(scope).into());
    if namespace_value.is_null_or_undefined() && name == name.to_ascii_lowercase() {
        let _ = call_object_method(
            scope,
            owner,
            "setAttribute",
            &[
                v8_string(scope, &name)
                    .map(Into::<v8::Local<'_, v8::Value>>::into)
                    .unwrap_or_else(|| v8::String::empty(scope).into()),
                v8_string(scope, &verified_value)
                    .map(Into::<v8::Local<'_, v8::Value>>::into)
                    .unwrap_or_else(|| v8::String::empty(scope).into()),
            ],
        );
    } else {
        let set_attribute_ns_result = call_object_method(
            scope,
            owner,
            "setAttributeNS",
            &[
                namespace_value,
                v8_string(scope, &name)
                    .map(Into::<v8::Local<'_, v8::Value>>::into)
                    .unwrap_or_else(|| v8::String::empty(scope).into()),
                v8_string(scope, &verified_value)
                    .map(Into::<v8::Local<'_, v8::Value>>::into)
                    .unwrap_or_else(|| v8::String::empty(scope).into()),
            ],
        );
        if set_attribute_ns_result.is_none() {
            let _ = call_object_method(
                scope,
                owner,
                "setAttribute",
                &[
                    v8_string(scope, &name)
                        .map(Into::<v8::Local<'_, v8::Value>>::into)
                        .unwrap_or_else(|| v8::String::empty(scope).into()),
                    v8_string(scope, &verified_value)
                        .map(Into::<v8::Local<'_, v8::Value>>::into)
                        .unwrap_or_else(|| v8::String::empty(scope).into()),
                ],
            );
        }
    }
}

fn set_attr_state_value(
    scope: &mut v8::PinScope<'_, '_>,
    state: v8::Local<'_, v8::Object>,
    value: &str,
) {
    let _ = state.set(
        scope,
        v8str(scope, "value").into(),
        v8_string(scope, value)
            .map(Into::<v8::Local<'_, v8::Value>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
    );
}

fn attr_value_setter_wrote_live_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> bool {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, owner) else {
        return false;
    };
    let namespace = attr_state_nullable_string(scope, state, "namespaceURI");
    let local_name = object_string_property(scope, state, "localName")
        .filter(|local_name| !local_name.is_empty())
        .or_else(|| name.rsplit_once(':').map(|(_, local)| local.to_owned()))
        .unwrap_or_else(|| name.to_owned());
    let prefix = attr_state_nullable_string(scope, state, "prefix");
    let normalized_name = unsafe { &*runtime_ptr }
        .dom_host()
        .dom()
        .normalized_attribute_name(handle, name);
    if namespace.is_none() && normalized_name.as_deref() == Some(name) {
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let _ = set_live_element_attribute_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                handle,
                name,
                value,
            );
        });
    } else {
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let _ = set_live_element_attribute_ns_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                handle,
                namespace.as_deref(),
                prefix.as_deref(),
                &local_name,
                name,
                value,
            );
        });
    }
    true
}

fn attr_value_setter_wrote_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> bool {
    let Some((runtime_ptr, _)) = detached_native_element_runtime_and_handle(scope, owner) else {
        return false;
    };
    let namespace = attr_state_nullable_string(scope, state, "namespaceURI");
    if namespace.is_none() {
        let name = detached_attribute_name(scope, owner, name);
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let _ = write_detached_native_attribute_appending_to_current_reaction_queue(
                scope, owner, &name, value,
            );
        });
        return true;
    }
    let local_name = object_string_property(scope, state, "localName")
        .filter(|local_name| !local_name.is_empty())
        .or_else(|| name.rsplit_once(':').map(|(_, local)| local.to_owned()))
        .unwrap_or_else(|| name.to_owned());
    let prefix = attr_state_nullable_string(scope, state, "prefix");
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = write_detached_native_attribute_ns_appending_to_current_reaction_queue(
            scope,
            owner,
            namespace.as_deref(),
            prefix.as_deref(),
            name,
            &local_name,
            value,
        );
    });
    true
}

fn attr_state_nullable_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<String> {
    let key = v8_string(scope, property)?;
    let value = state.get(scope, key.into())?;
    if value.is_null_or_undefined() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
}
