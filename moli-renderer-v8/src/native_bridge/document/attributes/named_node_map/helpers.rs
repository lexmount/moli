use super::cache::NAMED_NODE_MAP_ELEMENT_SLOT;
use super::*;
use crate::dom::native::Node;
use crate::native_bridge::document::{
    detached_attribute_name, detached_attributes_map, detached_map_has,
    read_detached_native_attribute_names, read_detached_native_attribute_snapshot,
};
use crate::util::get_private_object;

pub(super) fn named_node_map_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, map, NAMED_NODE_MAP_ELEMENT_SLOT).or_else(|| {
        map.get_internal_field(scope, 0)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    })
}

fn native_named_node_map_attribute_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<Vec<String>> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, element)
        && let Some(native_element) = unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
    {
        let names = native_element
            .attributes()
            .iter()
            .filter(|attribute| {
                !is_synthetic_detached_style_attribute(
                    scope,
                    element,
                    &attribute.name(),
                    attribute.value(),
                )
            })
            .map(|attribute| attribute.name())
            .collect::<Vec<_>>();
        return Some(names);
    }
    read_detached_native_attribute_names(scope, element).map(|names| {
        names
            .into_iter()
            .filter(|name| !is_synthetic_detached_style_name(scope, element, name))
            .collect()
    })
}

pub(super) fn named_node_map_attribute_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    if let Some(names) = native_named_node_map_attribute_names(scope, element) {
        return names;
    }
    fallback_named_node_map_attribute_names(scope, element)
}

fn fallback_named_node_map_attribute_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    let Some(names) = call_object_method(scope, element, "getAttributeNames", &[]) else {
        return Vec::new();
    };
    let Some(names) = names.to_object(scope) else {
        return Vec::new();
    };
    let Some(length) = names
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
    else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(length as usize);
    for index in 0..length {
        let Some(name) = names.get_index(scope, index) else {
            continue;
        };
        let Ok(name) = v8::Local::<v8::String>::try_from(name) else {
            continue;
        };
        out.push(name.to_rust_string_lossy(scope));
    }
    out
}

pub(super) struct IndexedAttribute {
    name: String,
    value: String,
    namespace_uri: Option<String>,
    prefix: Option<String>,
    local_name: String,
}

fn live_indexed_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    element: v8::Local<'_, v8::Object>,
    index: usize,
) -> Option<IndexedAttribute> {
    let (runtime_ptr, handle) = node_runtime_and_handle_from_object(scope, element).ok()?;
    let runtime = unsafe { &*runtime_ptr };
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    let attribute = element.attributes().get(index)?;
    Some(IndexedAttribute {
        name: attribute.name(),
        value: attribute.value().to_owned(),
        namespace_uri: (!attribute.namespace().is_empty())
            .then(|| attribute.namespace().to_owned()),
        prefix: attribute.prefix().map(str::to_owned),
        local_name: attribute.local_name().to_owned(),
    })
}

fn detached_indexed_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    index: usize,
) -> Option<IndexedAttribute> {
    let attribute = read_detached_native_attribute_snapshot(scope, element)?
        .into_iter()
        .filter(|attribute| {
            !is_synthetic_detached_style_attribute(
                scope,
                element,
                &attribute.name,
                &attribute.value,
            )
        })
        .nth(index)?;
    Some(IndexedAttribute {
        name: attribute.name,
        value: attribute.value,
        namespace_uri: attribute.namespace_uri,
        prefix: attribute.prefix,
        local_name: attribute.local_name,
    })
}

fn is_synthetic_detached_style_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> bool {
    name == "style"
        && detached_attributes_map(scope, element)
            .is_some_and(|attributes| !detached_map_has(scope, attributes, "style"))
}

fn is_synthetic_detached_style_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> bool {
    value.is_empty() && is_synthetic_detached_style_name(scope, element, name)
}

fn indexed_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    index: usize,
) -> Option<IndexedAttribute> {
    live_indexed_attribute(scope, element, index)
        .or_else(|| detached_indexed_attribute(scope, element, index))
}

fn indexed_attribute_cache_key(attribute: &IndexedAttribute) -> String {
    format!(
        "__indexed:{}\u{0}{}\u{0}{}\u{0}{}",
        attribute.namespace_uri.as_deref().unwrap_or_default(),
        attribute.prefix.as_deref().unwrap_or_default(),
        attribute.local_name,
        attribute.name
    )
}

fn indexed_attribute_can_alias_qualified_name(attribute: &IndexedAttribute) -> bool {
    attribute.namespace_uri.is_none() || attribute.prefix.is_some()
}

fn indexed_attribute_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    index: usize,
) -> Option<v8::Local<'s, v8::Object>> {
    let attribute = indexed_attribute(scope, element, index)?;
    let cache = live_attr_cache_object(scope, element)?;
    let cache_key = indexed_attribute_cache_key(&attribute);
    let namespace_key =
        namespace_attr_cache_key(attribute.namespace_uri.as_deref(), &attribute.local_name);
    // Qualified names are not unique across namespaces. Only canonical keys
    // identify an Attr; the name alias is an exposed lookup projection.
    if let Some(attr) = object_property_as_object(scope, cache, &cache_key)
        .or_else(|| object_property_as_object(scope, cache, &namespace_key))
    {
        if let Some(state) = attr_state_object(scope, attr) {
            let _ = state.set(scope, v8str(scope, "ownerElement").into(), element.into());
            let _ = state.set(
                scope,
                v8str(scope, "value").into(),
                v8_string(scope, &attribute.value)
                    .map(Into::<v8::Local<'_, v8::Value>>::into)
                    .unwrap_or_else(|| v8::String::empty(scope).into()),
            );
        }
        set_attr_cache_entry(scope, cache, &cache_key, attr);
        if indexed_attribute_can_alias_qualified_name(&attribute) {
            set_attr_cache_entry(scope, cache, &attribute.name, attr);
        }
        set_attr_cache_entry(scope, cache, &namespace_key, attr);
        return Some(attr);
    }
    let attr = new_attr_object(
        scope,
        &attribute.name,
        &attribute.value,
        Some(element),
        None,
        attribute.namespace_uri.as_deref(),
        attribute.prefix.as_deref(),
        &attribute.local_name,
    )?;
    set_attr_cache_entry(scope, cache, &cache_key, attr);
    if indexed_attribute_can_alias_qualified_name(&attribute) {
        set_attr_cache_entry(scope, cache, &attribute.name, attr);
    }
    set_attr_cache_entry(scope, cache, &namespace_key, attr);
    Some(attr)
}

fn fallback_attribute_node_for_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    index: usize,
) -> Option<v8::Local<'s, v8::Value>> {
    let names = fallback_named_node_map_attribute_names(scope, element);
    let name = names.get(index)?;
    let name_value = v8_string(scope, name)?;
    call_object_method(scope, element, "getAttributeNode", &[name_value.into()])
}

pub(super) fn attribute_node_for_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    index: usize,
) -> Option<v8::Local<'s, v8::Value>> {
    if native_named_node_map_attribute_names(scope, element).is_some() {
        return indexed_attribute_object(scope, element, index).map(Into::into);
    }
    indexed_attribute_object(scope, element, index)
        .map(Into::into)
        .or_else(|| fallback_attribute_node_for_index(scope, element, index))
}

fn fallback_attribute_node_for_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let name = v8_string(scope, name)?;
    call_object_method(scope, element, "getAttributeNode", &[name.into()])
}

pub(super) fn attribute_node_for_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let normalized = detached_attribute_name(scope, element, name);
    if let Some(names) = native_named_node_map_attribute_names(scope, element) {
        return names
            .iter()
            .position(|candidate| candidate == &normalized)
            .and_then(|index| attribute_node_for_index(scope, element, index));
    }
    fallback_attribute_node_for_name(scope, element, &normalized)
}

fn attribute_index_by_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    local_name: &str,
) -> Option<usize> {
    let namespace = namespace.unwrap_or_default();
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, element) {
        let runtime = unsafe { &*runtime_ptr };
        let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
        return element.attributes().iter().position(|attribute| {
            attribute.namespace() == namespace && attribute.local_name() == local_name
        });
    }
    read_detached_native_attribute_snapshot(scope, element)?
        .iter()
        .position(|attribute| {
            attribute.namespace_uri.as_deref().unwrap_or_default() == namespace
                && attribute.local_name == local_name
        })
}

pub(super) fn attribute_node_for_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    local_name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    attribute_index_by_namespace(scope, element, namespace, local_name)
        .and_then(|index| attribute_node_for_index(scope, element, index))
}

pub(super) fn reserved_named_node_map_key(key: &str) -> bool {
    matches!(
        key,
        "length"
            | "item"
            | "getNamedItem"
            | "setNamedItem"
            | "removeNamedItem"
            | "getNamedItemNS"
            | "setNamedItemNS"
            | "removeNamedItemNS"
            | "constructor"
            | "hasOwnProperty"
            | "isPrototypeOf"
            | "propertyIsEnumerable"
            | "toLocaleString"
            | "toString"
            | "valueOf"
    )
}
