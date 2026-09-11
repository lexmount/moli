use super::helpers::{
    remove_storage_prototype_index_descriptor, set_storage_prototype_index_descriptor,
    storage_internal_name_utf16, storage_key_is_shadowed_by_prototype,
    storage_prototype_index_descriptor, storage_put_utf16, storage_remove_utf16,
    with_storage_store,
};
use super::*;
use crate::util::{throw_type_error, v8_string_from_utf16_units, v8_string_to_u16_string};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct StorageValuePropertyDescriptorDeclaration<'scope> {
    value: v8::Local<'scope, v8::Value>,
    writable: bool,
    enumerable: bool,
    configurable: bool,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct StorageAccessorPropertyDescriptorDeclaration<'scope> {
    get: Option<v8::Local<'scope, v8::Value>>,
    set: Option<v8::Local<'scope, v8::Value>>,
    enumerable: bool,
    configurable: bool,
}

fn storage_value_to_dom_string_utf16(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<Vec<u16>> {
    if value.is_symbol() {
        throw_type_error(scope, "Failed to convert value to DOMString.");
        return None;
    }
    value
        .to_string(scope)
        .map(|value| v8_string_to_u16_string(scope, value).into_vec())
}

pub(super) fn storage_named_getter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok(key_string) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = v8_string_to_u16_string(scope, key_string).into_vec();
    if storage_internal_name_utf16(&key)
        || storage_key_is_shadowed_by_prototype(scope, args.holder(), key_string)
    {
        return v8::Intercepted::kNo;
    }
    let value = with_storage_store(scope, args.holder(), |store, origin| {
        store.get_item_utf16(origin, &key)
    })
    .flatten();
    let Some(value) = value else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = v8_string_from_utf16_units(scope, &value) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value.into());
    v8::Intercepted::kYes
}

pub(super) fn storage_named_setter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    value: v8::Local<'_, v8::Value>,
    args: v8::PropertyCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok(key_string) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = v8_string_to_u16_string(scope, key_string).into_vec();
    if storage_internal_name_utf16(&key) {
        return v8::Intercepted::kNo;
    }
    let Some(value) = storage_value_to_dom_string_utf16(scope, value) else {
        return v8::Intercepted::kNo;
    };
    let _ = storage_put_utf16(scope, args.holder(), &key, &value);
    v8::Intercepted::kYes
}

pub(super) fn storage_named_query(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok(key_string) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = v8_string_to_u16_string(scope, key_string).into_vec();
    if storage_internal_name_utf16(&key)
        || storage_key_is_shadowed_by_prototype(scope, args.holder(), key_string)
    {
        return v8::Intercepted::kNo;
    }
    let contains = with_storage_store(scope, args.holder(), |store, origin| {
        store.contains_key_utf16(origin, &key)
    })
    .unwrap_or(false);
    if !contains {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(super) fn storage_named_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok(key_string) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = v8_string_to_u16_string(scope, key_string).into_vec();
    if storage_internal_name_utf16(&key) {
        return v8::Intercepted::kNo;
    }
    let removed = storage_remove_utf16(scope, args.holder(), &key);
    rv.set_bool(removed);
    v8::Intercepted::kYes
}

pub(super) fn storage_named_enumerator(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let keys = with_storage_store(scope, args.holder(), |store, origin| {
        store.sorted_keys_utf16(origin)
    })
    .unwrap_or_default();
    let array = v8::Array::new(scope, keys.len() as i32);
    for (index, key) in keys.iter().enumerate() {
        if let Some(value) = v8_string_from_utf16_units(scope, key) {
            let _ = array.set_index(scope, index as u32, value.into());
        }
    }
    rv.set(array);
}

pub(super) fn storage_named_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok(key_string) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = v8_string_to_u16_string(scope, key_string).into_vec();
    if storage_internal_name_utf16(&key)
        || storage_key_is_shadowed_by_prototype(scope, args.holder(), key_string)
    {
        return v8::Intercepted::kNo;
    }
    let value = with_storage_store(scope, args.holder(), |store, origin| {
        store.get_item_utf16(origin, &key)
    })
    .flatten();
    let Some(value) = value else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = v8_string_from_utf16_units(scope, &value) else {
        return v8::Intercepted::kNo;
    };
    let Some(descriptor) = storage_value_property_descriptor(scope, value.into()) else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(super) fn storage_named_definer(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    desc: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok(key_string) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = v8_string_to_u16_string(scope, key_string).into_vec();
    if storage_internal_name_utf16(&key) || !desc.has_value() {
        return v8::Intercepted::kNo;
    }
    let Some(value) = storage_value_to_dom_string_utf16(scope, desc.value()) else {
        return v8::Intercepted::kNo;
    };
    let _ = storage_put_utf16(scope, args.holder(), &key, &value);
    v8::Intercepted::kYes
}

pub(super) fn storage_indexed_getter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    if storage_prototype_index_getter_value(scope, index, args.holder(), &mut rv) {
        return v8::Intercepted::kYes;
    }
    let Some(key) = v8_string(scope, &index.to_string()) else {
        return v8::Intercepted::kNo;
    };
    storage_named_getter(scope, key.into(), args, rv)
}

pub(super) fn storage_indexed_setter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    value: v8::Local<'_, v8::Value>,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(key) = v8_string(scope, &index.to_string()) else {
        return v8::Intercepted::kNo;
    };
    storage_named_setter(scope, key.into(), value, args, rv)
}

pub(super) fn storage_indexed_query(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if storage_prototype_index_descriptor(scope, index).is_some() {
        let mut rv = rv;
        rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    let Some(key) = v8_string(scope, &index.to_string()) else {
        return v8::Intercepted::kNo;
    };
    storage_named_query(scope, key.into(), args, rv)
}

pub(super) fn storage_indexed_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    if storage_prototype_index_descriptor(scope, index).is_some() {
        return v8::Intercepted::kNo;
    }
    let Some(key) = v8_string(scope, &index.to_string()) else {
        return v8::Intercepted::kNo;
    };
    storage_named_descriptor(scope, key.into(), args, rv)
}

pub(super) fn storage_indexed_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(key) = v8_string(scope, &index.to_string()) else {
        return v8::Intercepted::kNo;
    };
    storage_named_deleter(scope, key.into(), args, rv)
}

pub(super) fn storage_indexed_definer(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    desc: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(key) = v8_string(scope, &index.to_string()) else {
        return v8::Intercepted::kNo;
    };
    storage_named_definer(scope, key.into(), desc, args, rv)
}

pub(super) fn storage_prototype_indexed_getter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    if storage_prototype_index_getter_value(scope, index, args.holder(), &mut rv) {
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

pub(super) fn storage_prototype_indexed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    _args: v8::PropertyCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(descriptor) = storage_value_property_descriptor(scope, value) else {
        return v8::Intercepted::kNo;
    };
    if !set_storage_prototype_index_descriptor(scope, index, descriptor) {
        return v8::Intercepted::kNo;
    }
    v8::Intercepted::kYes
}

pub(super) fn storage_prototype_indexed_query(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Some(descriptor) = storage_prototype_index_descriptor(scope, index) else {
        return v8::Intercepted::kNo;
    };
    rv.set_int32(property_attributes_for_descriptor(scope, descriptor).as_u32() as i32);
    v8::Intercepted::kYes
}

pub(super) fn storage_prototype_indexed_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(remove_storage_prototype_index_descriptor(scope, index));
    v8::Intercepted::kYes
}

pub(super) fn storage_prototype_indexed_definer(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    desc: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(descriptor) = storage_descriptor_object_from_property_descriptor(scope, desc) else {
        return v8::Intercepted::kNo;
    };
    if !set_storage_prototype_index_descriptor(scope, index, descriptor) {
        return v8::Intercepted::kNo;
    }
    v8::Intercepted::kYes
}

pub(super) fn storage_prototype_indexed_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(descriptor) = storage_prototype_index_descriptor(scope, index) else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

fn storage_value_property_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    StorageValuePropertyDescriptorDeclaration::new(value, true, true, true)
        .bind(scope)
        .ok()
}

fn storage_descriptor_object_from_property_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    desc: &v8::PropertyDescriptor,
) -> Option<v8::Local<'s, v8::Object>> {
    if desc.has_value() {
        return StorageValuePropertyDescriptorDeclaration::new(
            storage_descriptor_local_value(scope, desc.value()),
            desc.has_writable() && desc.writable(),
            desc.has_enumerable() && desc.enumerable(),
            desc.has_configurable() && desc.configurable(),
        )
        .bind(scope)
        .ok();
    }
    StorageAccessorPropertyDescriptorDeclaration::new(
        desc.has_get()
            .then(|| storage_descriptor_local_value(scope, desc.get())),
        desc.has_set()
            .then(|| storage_descriptor_local_value(scope, desc.set())),
        desc.has_enumerable() && desc.enumerable(),
        desc.has_configurable() && desc.configurable(),
    )
    .bind(scope)
    .ok()
}

fn storage_descriptor_local_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'_, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let value = v8::Global::new(scope, value);
    v8::Local::new(scope, &value)
}

fn storage_prototype_index_getter_value(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    receiver: v8::Local<'_, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) -> bool {
    let Some(descriptor) = storage_prototype_index_descriptor(scope, index) else {
        return false;
    };
    if descriptor_has_own(scope, descriptor, "get")
        && let Some(getter) = descriptor.get(scope, v8str(scope, "get").into())
    {
        if getter.is_undefined() {
            rv.set(v8::undefined(scope).into());
            return true;
        }
        if let Ok(getter) = v8::Local::<v8::Function>::try_from(getter) {
            if let Some(value) = getter.call(scope, receiver.into(), &[]) {
                rv.set(value);
            }
            return true;
        }
    }
    if descriptor_has_own(scope, descriptor, "value")
        && let Some(value) = descriptor.get(scope, v8str(scope, "value").into())
    {
        rv.set(value);
        return true;
    }
    rv.set(v8::undefined(scope).into());
    true
}

fn property_attributes_for_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    descriptor: v8::Local<'_, v8::Object>,
) -> v8::PropertyAttribute {
    let mut attributes = v8::PropertyAttribute::NONE;
    let read_only = if descriptor_has_own(scope, descriptor, "value") {
        !descriptor_bool(scope, descriptor, "writable")
    } else {
        descriptor
            .get(scope, v8str(scope, "set").into())
            .is_none_or(|setter| setter.is_undefined())
    };
    if read_only {
        attributes = attributes | v8::PropertyAttribute::READ_ONLY;
    }
    if !descriptor_bool(scope, descriptor, "enumerable") {
        attributes = attributes | v8::PropertyAttribute::DONT_ENUM;
    }
    if !descriptor_bool(scope, descriptor, "configurable") {
        attributes = attributes | v8::PropertyAttribute::DONT_DELETE;
    }
    attributes
}

fn descriptor_bool(
    scope: &mut v8::PinScope<'_, '_>,
    descriptor: v8::Local<'_, v8::Object>,
    key: &'static str,
) -> bool {
    descriptor
        .get(scope, v8str(scope, key).into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn descriptor_has_own(
    scope: &mut v8::PinScope<'_, '_>,
    descriptor: v8::Local<'_, v8::Object>,
    key: &'static str,
) -> bool {
    descriptor
        .has_own_property(scope, v8str(scope, key).into())
        .unwrap_or(false)
}
