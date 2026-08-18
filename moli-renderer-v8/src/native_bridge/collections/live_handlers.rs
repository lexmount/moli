use super::*;
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

pub(in crate::native_bridge::collections) fn live_collection_indexed_getter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let handles = descriptor.resolve(unsafe { &*runtime_ptr });
    let Some(handle) = handles.get(index as usize).copied() else {
        return v8::Intercepted::kNo;
    };
    let Some(node) = wrapped_handle_value(scope, runtime_ptr, handle) else {
        return v8::Intercepted::kNo;
    };
    rv.set(node);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_indexed_setter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    value: v8::Local<'_, v8::Value>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok((_runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if descriptor.collection_kind == CollectionKind::OptionsCollection {
        return options_collection_indexed_setter(scope, index, value, args, rv);
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_indexed_query(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if descriptor.resolve(unsafe { &*runtime_ptr }).len() <= index as usize {
        return v8::Intercepted::kNo;
    }
    let attributes = if descriptor.collection_kind == CollectionKind::OptionsCollection {
        v8::PropertyAttribute::NONE
    } else {
        v8::PropertyAttribute::READ_ONLY
    };
    rv.set_int32(attributes.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_indexed_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if descriptor.resolve(unsafe { &*runtime_ptr }).len() <= index as usize {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_indexed_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Some(handle) = descriptor
        .resolve(unsafe { &*runtime_ptr })
        .get(index as usize)
        .copied()
    else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = wrapped_handle_value(scope, runtime_ptr, handle) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(
        value,
        descriptor.collection_kind == CollectionKind::OptionsCollection,
        true,
    )
    .bind(scope) else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_indexed_definer(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, collection)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if collection.collection_kind != CollectionKind::OptionsCollection {
        rv.set_bool(false);
        return v8::Intercepted::kYes;
    }
    if descriptor.has_get() || descriptor.has_set() {
        rv.set_bool(false);
        return v8::Intercepted::kYes;
    }
    let value = if descriptor.has_value() {
        descriptor.value()
    } else {
        v8::undefined(scope).into()
    };
    if set_select_indexed_option(scope, runtime_ptr, collection.root, index, value) {
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}

pub(in crate::native_bridge::collections) fn live_collection_indexed_enumerator(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys = (0..descriptor.resolve(unsafe { &*runtime_ptr }).len())
        .map(|index| v8::Integer::new_from_unsigned(scope, index as u32).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

pub(in crate::native_bridge::collections) fn live_collection_named_enumerator(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys = named_item_property_names(unsafe { &*runtime_ptr }, &descriptor)
        .into_iter()
        .filter_map(|key| v8_string(scope, &key).map(Into::into))
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

pub(in crate::native_bridge::collections) fn live_collection_named_getter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if !matches!(
        descriptor.collection_kind,
        CollectionKind::HtmlCollection
            | CollectionKind::FormControlsCollection
            | CollectionKind::OptionsCollection
    ) {
        return v8::Intercepted::kNo;
    }
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if is_array_index_property_name(&key) || key == "length" {
        return v8::Intercepted::kNo;
    };
    let Some(value) = live_collection_named_value(scope, runtime_ptr, &descriptor, key) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_named_query(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if !matches!(
        descriptor.collection_kind,
        CollectionKind::HtmlCollection
            | CollectionKind::FormControlsCollection
            | CollectionKind::OptionsCollection
    ) {
        return v8::Intercepted::kNo;
    }
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if is_array_index_property_name(&key) || key == "length" {
        return v8::Intercepted::kNo;
    };
    if named_item_matches(unsafe { &*runtime_ptr }, &descriptor, &key).is_empty() {
        return v8::Intercepted::kNo;
    }
    // Direct writes to a supported name are rejected by the setter interceptor.
    // Reporting READ_ONLY here would also make V8 reject writes whose receiver is
    // an ordinary object inheriting from this collection, instead of creating the
    // receiver's own property as WebIDL requires.
    rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_named_setter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    _value: v8::Local<'_, v8::Value>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    if !live_collection_has_named_property(scope, key, args.holder()) {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_named_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if !live_collection_has_named_property(scope, key, args.holder()) {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_named_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if !live_collection_has_named_properties(&descriptor) {
        return v8::Intercepted::kNo;
    }
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if is_array_index_property_name(&key) || key == "length" {
        return v8::Intercepted::kNo;
    }
    let Some(value) = live_collection_named_value(scope, runtime_ptr, &descriptor, key) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, false, false).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn live_collection_named_definer(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    _descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    if !live_collection_has_named_property(scope, key, args.holder()) {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn live_collection_has_named_properties(descriptor: &LiveCollectionDescriptor) -> bool {
    matches!(
        descriptor.collection_kind,
        CollectionKind::HtmlCollection
            | CollectionKind::FormControlsCollection
            | CollectionKind::OptionsCollection
    )
}

fn live_collection_has_named_property(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    holder: v8::Local<'_, v8::Object>,
) -> bool {
    let Ok((runtime_ptr, descriptor)) = live_collection_descriptor_from_object(scope, holder)
    else {
        return false;
    };
    if !live_collection_has_named_properties(&descriptor) {
        return false;
    }
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return false;
    };
    let key = key.to_rust_string_lossy(scope);
    !is_array_index_property_name(&key)
        && key != "length"
        && !named_item_matches(unsafe { &*runtime_ptr }, &descriptor, &key).is_empty()
}

fn live_collection_named_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    descriptor: &LiveCollectionDescriptor,
    key: String,
) -> Option<v8::Local<'s, v8::Value>> {
    let runtime = unsafe { &mut *runtime_ptr };
    let matches = named_item_matches(runtime, descriptor, &key);
    if descriptor.collection_kind == CollectionKind::FormControlsCollection && matches.len() > 1 {
        let live_descriptor = LiveCollectionDescriptor {
            collection_kind: CollectionKind::RadioNodeList,
            query_kind: LiveCollectionQueryKind::FormControlsByName,
            root: descriptor.root,
            query: Some(key),
            include_root: false,
            tag_name_html_document: None,
            resolution_cache: Default::default(),
        };
        return Some(build_live_collection_wrapper(scope, runtime_ptr, live_descriptor).into());
    }
    let handle = matches.first().copied()?;
    wrapped_handle_value(scope, runtime_ptr, handle)
}

pub(in crate::native_bridge::collections) fn static_handle_collection_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let (runtime_ptr, collection_id) =
        static_handle_collection_id_from_object(scope, args.holder())
            .expect("handle-backed static NodeList must retain its collection id");
    let Some(handle) =
        static_handle_collection_handle_at(runtime_ptr, collection_id, index as usize)
    else {
        return v8::Intercepted::kNo;
    };
    let node = wrapped_handle_value(scope, runtime_ptr, handle).unwrap_or_else(|| {
        panic!("failed to materialize handle-backed static NodeList index `{index}`")
    });
    rv.set(node);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn static_handle_collection_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let (runtime_ptr, collection_id) =
        static_handle_collection_id_from_object(scope, args.holder())
            .expect("handle-backed static NodeList must retain its collection id");
    let length = static_handle_collection_len(runtime_ptr, collection_id)
        .expect("handle-backed static NodeList id must resolve to its handle store");
    if length <= index as usize {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn static_handle_collection_indexed_setter(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _value: v8::Local<'_, v8::Value>,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn static_handle_collection_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let (runtime_ptr, collection_id) =
        static_handle_collection_id_from_object(scope, args.holder())
            .expect("handle-backed static NodeList must retain its collection id");
    let length = static_handle_collection_len(runtime_ptr, collection_id)
        .expect("handle-backed static NodeList id must resolve to its handle store");
    if length <= index as usize {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn static_handle_collection_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let (runtime_ptr, collection_id) =
        static_handle_collection_id_from_object(scope, args.holder())
            .expect("handle-backed static NodeList must retain its collection id");
    let Some(handle) =
        static_handle_collection_handle_at(runtime_ptr, collection_id, index as usize)
    else {
        return v8::Intercepted::kNo;
    };
    let value = wrapped_handle_value(scope, runtime_ptr, handle).unwrap_or_else(|| {
        panic!("failed to materialize handle-backed static NodeList index `{index}`")
    });
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, false, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn static_handle_collection_indexed_definer(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge::collections) fn static_handle_collection_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let (runtime_ptr, collection_id) =
        static_handle_collection_id_from_object(scope, args.holder())
            .expect("handle-backed static NodeList must retain its collection id");
    let length = static_handle_collection_len(runtime_ptr, collection_id)
        .expect("handle-backed static NodeList id must resolve to its handle store");
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index as u32).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}
