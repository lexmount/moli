use super::*;
use indexmap::IndexSet;

pub(in crate::native_bridge::collections) fn callback_arg_live_collection_kind(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
) -> Option<LiveCollectionQueryKind> {
    match callback_arg_string(scope, args, index).as_deref() {
        Some("childNodes") => Some(LiveCollectionQueryKind::ChildNodes),
        Some("children") => Some(LiveCollectionQueryKind::Children),
        Some("formControls") => Some(LiveCollectionQueryKind::FormControls),
        Some("options") => Some(LiveCollectionQueryKind::Options),
        Some("selectedOptions") => Some(LiveCollectionQueryKind::SelectedOptions),
        Some("tagName") => Some(LiveCollectionQueryKind::TagName),
        Some("tagNameNs") => Some(LiveCollectionQueryKind::TagNameNs),
        Some("className") => Some(LiveCollectionQueryKind::ClassName),
        Some("name") => Some(LiveCollectionQueryKind::Name),
        Some("forms") => Some(LiveCollectionQueryKind::Forms),
        Some("images") => Some(LiveCollectionQueryKind::Images),
        Some("scripts") => Some(LiveCollectionQueryKind::Scripts),
        Some("links") => Some(LiveCollectionQueryKind::Links),
        Some("anchors") => Some(LiveCollectionQueryKind::Anchors),
        Some("embeds") => Some(LiveCollectionQueryKind::Embeds),
        Some("applets") => Some(LiveCollectionQueryKind::Applets),
        _ => None,
    }
}

pub(in crate::native_bridge::collections) fn live_collection_descriptor_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> std::result::Result<(*mut JsContextHost, LiveCollectionDescriptor), String> {
    let runtime_ptr = runtime_ptr_from_object(scope, object)?;
    let collection_id = object_collection_id(scope, object)?;
    let descriptor = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .live_collection_descriptor(collection_id)
        .cloned()
        .ok_or_else(|| format!("missing live collection descriptor `{collection_id}`"))?;
    Ok((runtime_ptr, descriptor))
}

pub(in crate::native_bridge::collections) fn static_handle_collection_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> std::result::Result<(*mut JsContextHost, u32), String> {
    let runtime_ptr = runtime_ptr_from_object(scope, object)?;
    let value = object
        .get_internal_field(scope, STATIC_HANDLE_COLLECTION_ID_INTERNAL_FIELD)
        .ok_or_else(|| "static handle collection missing id field".to_owned())?;
    let value = v8::Local::<v8::Value>::try_from(value)
        .map_err(|_| "static handle collection id field had invalid type".to_owned())?;
    let number = value
        .number_value(scope)
        .ok_or_else(|| "static handle collection id was not numeric".to_owned())?;
    if !number.is_finite() || number < 0.0 || number.fract() != 0.0 {
        return Err("static handle collection id was invalid".to_owned());
    }
    let collection_id = number as u32;
    Ok((runtime_ptr, collection_id))
}

pub(in crate::native_bridge::collections) fn static_handle_collection_len(
    runtime_ptr: *mut JsContextHost,
    collection_id: u32,
) -> Option<usize> {
    unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .static_handle_collection_len(collection_id)
}

pub(in crate::native_bridge::collections) fn static_handle_collection_handle_at(
    runtime_ptr: *mut JsContextHost,
    collection_id: u32,
    index: usize,
) -> Option<DomHandle> {
    unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .static_handle_collection_handle_at(collection_id, index)
}

pub(in crate::native_bridge::collections) fn object_collection_id(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> std::result::Result<u32, String> {
    let value = object
        .get_internal_field(scope, 1)
        .ok_or_else(|| "live collection missing descriptor field".to_owned())?;
    let value = v8::Local::<v8::Value>::try_from(value)
        .map_err(|_| "live collection descriptor field had invalid type".to_owned())?;
    let number = value
        .number_value(scope)
        .ok_or_else(|| "live collection descriptor field was not numeric".to_owned())?;
    if !number.is_finite() || number < 0.0 || number.fract() != 0.0 {
        return Err("live collection descriptor field was invalid".to_owned());
    }
    Ok(number as u32)
}

pub(in crate::native_bridge::collections) fn named_item_matches(
    host: &JsContextHost,
    descriptor: &LiveCollectionDescriptor,
    key: &str,
) -> Vec<DomHandle> {
    if !matches!(
        descriptor.collection_kind,
        CollectionKind::HtmlCollection
            | CollectionKind::FormControlsCollection
            | CollectionKind::OptionsCollection
    ) {
        return Vec::new();
    }

    descriptor
        .resolve(host)
        .iter()
        .copied()
        .filter(|handle| {
            host.dom_host()
                .node(*handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.matches_named_item_key(key))
        })
        .collect()
}

pub(in crate::native_bridge::collections) fn named_item_property_names(
    host: &JsContextHost,
    descriptor: &LiveCollectionDescriptor,
) -> Vec<String> {
    if !matches!(
        descriptor.collection_kind,
        CollectionKind::HtmlCollection
            | CollectionKind::FormControlsCollection
            | CollectionKind::OptionsCollection
    ) {
        return Vec::new();
    }

    let mut names = IndexSet::new();
    for handle in descriptor.resolve(host).iter().copied() {
        let Some(element) = host.dom_host().node(handle).and_then(Node::as_element) else {
            continue;
        };
        for value in [
            element.id(),
            (element.namespace() == "http://www.w3.org/1999/xhtml")
                .then(|| element.name_attribute())
                .flatten(),
        ]
        .into_iter()
        .flatten()
        {
            if value == "length" || is_array_index_property_name(value) {
                continue;
            }
            names.insert(value.to_owned());
        }
    }
    names.into_iter().collect()
}

pub(in crate::native_bridge) fn array_index_property_name(value: &str) -> Option<u32> {
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

pub(in crate::native_bridge::collections) fn is_array_index_property_name(value: &str) -> bool {
    array_index_property_name(value).is_some()
}
