//! WebAssembly descriptor adaptation and type reflection values.

use moli_v8_util::{v8_string, v8str};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain)]
pub(super) struct WebAssemblyMemoryNativeDescriptorDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    initial: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    shared: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub(super) struct WebAssemblyMemoryTypeDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    minimum: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: Option<v8::Local<'scope, v8::Value>>,
    #[webapi(data_property, enumerable)]
    shared: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub(super) struct WebAssemblyTableNativeDescriptorDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    element: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    initial: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub(super) struct WebAssemblyTableTypeDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    minimum: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    maximum: Option<v8::Local<'scope, v8::Value>>,
    #[webapi(data_property, enumerable)]
    element: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub(super) struct WebAssemblyGlobalDescriptorDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    mutable: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    value: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub(super) struct WebAssemblyTagTypeDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    parameters: Option<v8::Local<'scope, v8::Array>>,
}

pub(super) fn is_dictionary_like(value: v8::Local<'_, v8::Value>) -> bool {
    !value.is_null_or_undefined() && (value.is_object() || value.is_function())
}

pub(super) fn object_from_dictionary_like<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !is_dictionary_like(value) {
        return None;
    }
    v8::Local::<v8::Object>::try_from(value).ok()
}

pub(super) fn object_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> v8::Local<'s, v8::Value> {
    object
        .get(scope, v8str(scope, name).into())
        .unwrap_or_else(|| v8::undefined(scope).into())
}

pub(super) fn has_defined_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
) -> bool {
    let key = v8str(scope, name);
    object.has(scope, key.into()).unwrap_or(false)
        && object
            .get(scope, key.into())
            .is_some_and(|value| !value.is_undefined())
}

pub(super) fn string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
) -> v8::Local<'s, v8::Value> {
    v8_string(scope, value)
        .map(Into::into)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

pub(super) fn value_type_for_native_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    public_value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let value_string = public_value.to_string(scope)?;
    if value_string.to_rust_string_lossy(scope) == "funcref" {
        Some(string_value(scope, "anyfunc"))
    } else {
        Some(value_string.into())
    }
}

fn string_value_equals(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    expected: &str,
) -> bool {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope) == expected)
        .unwrap_or(false)
}

pub(super) fn descriptor_value_is_anyfunc<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    descriptor: v8::Local<'s, v8::Value>,
) -> bool {
    v8::Local::<v8::Object>::try_from(descriptor)
        .ok()
        .map(|descriptor| object_property(scope, descriptor, "value"))
        .is_some_and(|value| string_value_equals(scope, value, "anyfunc"))
}

pub(super) fn array_from_value_or_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Array>> {
    if value.is_null_or_undefined() {
        return Some(v8::Array::new(scope, 0));
    }
    // V8's WebAssembly.Tag constructor consumes `parameters` as an
    // array-like object (length plus indexed values), not a WebIDL sequence.
    // Mirror that native interpretation without consulting public Array.from.
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let length = object
        .get(scope, v8str(scope, "length").into())?
        .uint32_value(scope)?;
    let mut values = Vec::with_capacity(length as usize);
    for index in 0..length {
        values.push(object.get_index(scope, index)?);
    }
    Some(v8::Array::new_with_elements(scope, &values))
}

pub(super) fn clone_memory_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stored: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let minimum = stored.get(scope, v8str(scope, "minimum").into())?;
    if minimum.is_undefined() {
        return None;
    }
    let maximum = own_optional_property(scope, stored, "maximum");
    let shared = own_optional_property(scope, stored, "shared");
    WebAssemblyMemoryTypeDeclaration::new(minimum, maximum, shared)
        .bind(scope)
        .ok()
}

pub(super) fn clone_table_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stored: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let minimum = stored.get(scope, v8str(scope, "minimum").into())?;
    if minimum.is_undefined() {
        return None;
    }
    let element = stored.get(scope, v8str(scope, "element").into())?;
    if element.is_undefined() {
        return None;
    }
    let maximum = own_optional_property(scope, stored, "maximum");
    WebAssemblyTableTypeDeclaration::new(minimum, maximum, element)
        .bind(scope)
        .ok()
}

pub(super) fn clone_global_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    stored: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let r#mutable = stored.get(scope, v8str(scope, "mutable").into())?;
    if r#mutable.is_undefined() {
        return None;
    }
    let value = stored.get(scope, v8str(scope, "value").into())?;
    if value.is_undefined() {
        return None;
    }
    WebAssemblyGlobalDescriptorDeclaration::new(r#mutable, value)
        .bind(scope)
        .ok()
}

fn own_optional_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8str(scope, name);
    if snapshot.has_own_property(scope, key.into()) != Some(true) {
        return None;
    }
    snapshot
        .get(scope, key.into())
        .filter(|value| !value.is_undefined())
}

pub(super) fn clone_tag_type_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let parameters = own_optional_property(scope, snapshot, "parameters")
        .and_then(|value| array_from_value_or_empty(scope, value));
    WebAssemblyTagTypeDeclaration::new(parameters)
        .bind(scope)
        .expect("WebAssembly Tag type declaration should bind")
}
