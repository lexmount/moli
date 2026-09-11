use super::*;
pub(in crate::native_bridge::document) use crate::util::{
    call_object_method, get_property as object_property_value,
    object_defined_string_property as object_string_property, object_property_as_object,
};
use moli_webapi_declare::WebApiObject;

#[derive(Default, WebApiObject)]
#[webapi(record)]
struct ObjectArrayItemMethodDeclaration {
    #[webapi(method, callback = object_array_item_callback)]
    item: (),
}

pub(in crate::native_bridge::document) fn object_node_type(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<i32> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) {
        return unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .map(|node| node.node_type() as i32);
    }
    object
        .get(scope, v8_string(scope, "nodeType")?.into())?
        .int32_value(scope)
}

pub(in crate::native_bridge::document) fn array_index_property_name(value: &str) -> Option<u32> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return None;
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let index = value.parse::<u64>().ok()?;
    if index >= u64::from(u32::MAX) {
        return None;
    }
    u32::try_from(index).ok()
}

pub(in crate::native_bridge::document) fn object_is_shadow_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, object) {
        return unsafe { &*runtime_ptr }.dom_host().is_shadow_root(handle);
    }
    detached_state_kind(scope, object).as_deref() == Some("shadowRoot")
}

pub(in crate::native_bridge::document) fn object_dom_identity(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<(usize, DomHandle)> {
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object(scope, object).ok()?;
    Some((runtime_ptr as usize, handle))
}

pub(in crate::native_bridge::document) fn object_child_nodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(children) = object.get(scope, v8str(scope, "childNodes").into()) else {
        return Vec::new();
    };
    let Some(children) = children.to_object(scope) else {
        return Vec::new();
    };
    let Some(length_value) = children.get(scope, v8str(scope, "length").into()) else {
        return Vec::new();
    };
    let Some(length) = length_value.uint32_value(scope) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(length as usize);
    for index in 0..length {
        let Some(child) = children.get_index(scope, index) else {
            continue;
        };
        let Ok(child) = v8::Local::<v8::Object>::try_from(child) else {
            continue;
        };
        out.push(child);
    }
    out
}

pub(in crate::native_bridge::document) fn indexed_object_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(length_value) = object.get(scope, v8str(scope, "length").into()) else {
        return Vec::new();
    };
    let Some(length) = length_value.uint32_value(scope) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(length as usize);
    for index in 0..length {
        let Some(value) = object.get_index(scope, index) else {
            continue;
        };
        let Ok(value) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        out.push(value);
    }
    out
}

pub(in crate::native_bridge::document) fn object_array_item_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let index = args
        .get(0)
        .uint32_value(scope)
        .filter(|_| !args.get(0).is_null_or_undefined())
        .unwrap_or(u32::MAX);
    match args.this().get_index(scope, index) {
        Some(value) if !value.is_null_or_undefined() => rv.set(value),
        _ => rv.set_null(),
    }
}

pub(in crate::native_bridge::document) fn build_object_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[v8::Local<'s, v8::Object>],
) -> v8::Local<'s, v8::Array> {
    let array =
        crate::util::serialize_v8_array(scope, values).unwrap_or_else(|| v8::Array::new(scope, 0));
    install_object_array_item_method(scope, array.into());
    array
}

pub(in crate::native_bridge::document) fn install_object_array_item_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let _ = ObjectArrayItemMethodDeclaration::default().initialize(scope, object);
}
