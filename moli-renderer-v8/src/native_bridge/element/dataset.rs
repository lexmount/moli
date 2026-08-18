use super::super::{
    BridgeHandle, JsContextHost, bridge_handle_from_object,
    node::{node_is_element, node_runtime_and_handle_from_object_or_detached},
    throw_dom_exception, validate_attribute_name,
};
use super::{
    element_attribute, element_attribute_names, property_string_value,
    remove_live_element_attribute_ns_appending_to_current_reaction_queue,
    set_live_element_attribute_ns_appending_to_current_reaction_queue,
};
use crate::{custom_elements, document_runtime::DomHandle, util::v8_string};
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

pub(in crate::native_bridge) fn build_dom_string_map_wrapper_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let _ = template.set_internal_field_count(2);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(dataset_indexed_getter)
            .query(dataset_indexed_query)
            .setter(dataset_indexed_setter)
            .deleter(dataset_indexed_deleter)
            .enumerator(dataset_indexed_enumerator)
            .definer(dataset_indexed_definer)
            .descriptor(dataset_indexed_descriptor),
    );
    template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(dataset_named_getter)
            .query(dataset_named_query)
            .setter(dataset_named_setter)
            .deleter(dataset_named_deleter)
            .enumerator(dataset_named_enumerator)
            .definer(dataset_named_definer)
            .descriptor(dataset_named_descriptor)
            .flags(v8::PropertyHandlerFlags::ONLY_INTERCEPT_STRINGS),
    );
    template
}

fn dataset_runtime_and_handle_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> std::result::Result<(*mut JsContextHost, DomHandle), String> {
    let (runtime_ptr, handle) = bridge_handle_from_object(scope, object)?;
    match handle {
        BridgeHandle::Dataset(handle) => Ok((runtime_ptr, handle)),
        BridgeHandle::Window
        | BridgeHandle::Node(_)
        | BridgeHandle::ClassList(_, _)
        | BridgeHandle::Style(_)
        | BridgeHandle::ComputedStyle(_, _) => {
            Err("wrapper did not contain a DOMStringMap identity".to_owned())
        }
    }
}

fn dataset_attribute_name_to_key(attribute_name: &str) -> Option<String> {
    let suffix = attribute_name.strip_prefix("data-")?;
    if suffix.chars().any(|ch| ch.is_ascii_uppercase()) {
        return None;
    }
    let mut key = String::new();
    let mut chars = suffix.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '-' && chars.peek().is_some_and(char::is_ascii_lowercase) {
            let next = chars.next().expect("peeked dataset character should exist");
            key.push(next.to_ascii_uppercase());
        } else {
            key.push(ch);
        }
    }
    Some(key)
}

fn dataset_key_to_attribute_name(key: &str) -> String {
    let mut attribute = String::from("data-");
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            attribute.push('-');
            attribute.push(ch.to_ascii_lowercase());
        } else {
            attribute.push(ch);
        }
    }
    attribute
}

fn validate_dataset_key_for_write(
    key: &str,
) -> std::result::Result<(), (&'static str, i32, &'static str)> {
    if dataset_key_has_forbidden_dash_lowercase(key) {
        return Err((
            "SyntaxError",
            12,
            "DOMStringMap property names must not contain '-' followed by a lowercase ASCII letter.",
        ));
    }
    if !validate_attribute_name(&dataset_key_to_attribute_name(key)) {
        return Err((
            "InvalidCharacterError",
            5,
            "DOMStringMap property name maps to an invalid attribute name.",
        ));
    }
    Ok(())
}

fn dataset_key_has_forbidden_dash_lowercase(key: &str) -> bool {
    let mut chars = key.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '-' && chars.peek().is_some_and(char::is_ascii_lowercase) {
            return true;
        }
    }
    false
}

fn dataset_attribute_name_for_key(
    runtime: &JsContextHost,
    handle: DomHandle,
    key: &str,
) -> Option<String> {
    element_attribute_names(runtime, handle)
        .into_iter()
        .find(|name| dataset_attribute_name_to_key(name).as_deref() == Some(key))
}

fn dataset_property_names(runtime: &JsContextHost, handle: DomHandle) -> Vec<String> {
    element_attribute_names(runtime, handle)
        .into_iter()
        .filter_map(|name| dataset_attribute_name_to_key(&name))
        .collect()
}

fn dataset_property_index(key: &str) -> Option<u32> {
    let index = key.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == key).then_some(index)
}

fn dataset_index_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
) -> Option<v8::Local<'s, v8::Name>> {
    v8_string(scope, &index.to_string()).map(Into::into)
}

fn dataset_indexed_getter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(key) = dataset_index_name(scope, index) else {
        return v8::Intercepted::kNo;
    };
    dataset_named_getter(scope, key, args, rv)
}

fn dataset_indexed_query(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Some(key) = dataset_index_name(scope, index) else {
        return v8::Intercepted::kNo;
    };
    dataset_named_query(scope, key, args, rv)
}

fn dataset_indexed_setter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    value: v8::Local<'_, v8::Value>,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Some(key) = dataset_index_name(scope, index) else {
        return v8::Intercepted::kNo;
    };
    dataset_named_setter(scope, key, value, args, rv)
}

fn dataset_indexed_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(key) = dataset_index_name(scope, index) else {
        return v8::Intercepted::kNo;
    };
    dataset_named_deleter(scope, key, args, rv)
}

fn dataset_indexed_definer(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Some(key) = dataset_index_name(scope, index) else {
        return v8::Intercepted::kNo;
    };
    dataset_named_definer(scope, key, descriptor, args, rv)
}

fn dataset_indexed_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(key) = dataset_index_name(scope, index) else {
        return v8::Intercepted::kNo;
    };
    dataset_named_descriptor(scope, key, args, rv)
}

fn dataset_indexed_enumerator(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Ok((runtime_ptr, handle)) = dataset_runtime_and_handle_from_object(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys = dataset_property_names(unsafe { &*runtime_ptr }, handle)
        .into_iter()
        .filter_map(|key| dataset_property_index(&key))
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn set_dataset_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    value: &str,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = set_live_element_attribute_ns_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            None,
            None,
            name,
            name,
            value,
        );
    });
}

fn remove_dataset_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = remove_live_element_attribute_ns_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            None,
            name,
        );
    });
}

fn dataset_named_getter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = dataset_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    let runtime = unsafe { &*runtime_ptr };
    let Some(attribute_name) = dataset_attribute_name_for_key(runtime, handle, &key) else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = element_attribute(runtime, handle, &attribute_name) else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = v8_string(scope, &value) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value.into());
    v8::Intercepted::kYes
}

fn dataset_named_query(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = dataset_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if dataset_attribute_name_for_key(unsafe { &*runtime_ptr }, handle, &key).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
    v8::Intercepted::kYes
}

fn dataset_named_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = dataset_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    let runtime = unsafe { &*runtime_ptr };
    let Some(attribute_name) = dataset_attribute_name_for_key(runtime, handle, &key) else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = element_attribute(runtime, handle, &attribute_name) else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = v8_string(scope, &value) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) =
        DataPropertyDescriptorDeclaration::new(value.into(), true, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

fn dataset_named_setter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    value: v8::Local<'_, v8::Value>,
    args: v8::PropertyCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = dataset_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if let Err((name, code, message)) = validate_dataset_key_for_write(&key) {
        throw_dom_exception(scope, name, code, message);
        return v8::Intercepted::kYes;
    }
    let Some(value) = property_string_value(scope, value) else {
        return v8::Intercepted::kNo;
    };
    let attribute_name = dataset_key_to_attribute_name(&key);
    set_dataset_attribute(scope, runtime_ptr, handle, &attribute_name, &value);
    v8::Intercepted::kYes
}

fn dataset_named_definer(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    if descriptor.has_get() || descriptor.has_set() {
        rv.set_bool(false);
        return v8::Intercepted::kYes;
    }
    let value = if descriptor.has_value() {
        descriptor.value()
    } else {
        v8::undefined(scope).into()
    };
    dataset_named_setter(scope, key, value, args, rv)
}

fn dataset_named_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = dataset_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if !dataset_key_has_forbidden_dash_lowercase(&key) {
        let attribute_name = dataset_key_to_attribute_name(&key);
        remove_dataset_attribute(scope, runtime_ptr, handle, &attribute_name);
    }
    // `DOMStringMap` follows ordinary JS delete semantics here: deleting a property reports
    // whether the delete operation itself is allowed, not whether an HTML attribute happened to
    // exist beforehand. Browsers therefore return `true` for `delete el.dataset.foo` even when
    // `data-foo` was already absent.
    rv.set(v8::Boolean::new(scope, true));
    v8::Intercepted::kYes
}

fn dataset_named_enumerator(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Ok((runtime_ptr, handle)) = dataset_runtime_and_handle_from_object(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys = dataset_property_names(unsafe { &*runtime_ptr }, handle)
        .into_iter()
        .filter(|key| dataset_property_index(key).is_none())
        .filter_map(|key| v8_string(scope, &key).map(Into::into))
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

pub(in crate::native_bridge) fn node_dataset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    match runtime
        .native_bridge_mut()
        .wrap_dataset(scope, runtime_ptr, handle)
    {
        Some(dataset) => rv.set(dataset.into()),
        None => rv.set_null(),
    }
}
