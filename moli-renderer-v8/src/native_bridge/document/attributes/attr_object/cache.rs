use super::super::attribute_node::{native_attr_object_by_name, native_attr_object_by_namespace};
use super::instance::attr_current_value;
use super::*;
use crate::util::{get_private_value, new_null_prototype_object, set_private_value};

const ATTR_OBJECT_CACHE_SLOT: &str = "__moliAttrObjectCache";

fn ensure_object_cache<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(cache) = get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(cache);
    }
    let cache = new_null_prototype_object(scope);
    set_private_value(scope, object, slot, cache.into());
    Some(cache)
}

pub(in crate::native_bridge::document) fn live_attr_cache_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    ensure_object_cache(scope, element, ATTR_OBJECT_CACHE_SLOT)
}

pub(in crate::native_bridge::document) fn namespace_attr_cache_key(
    namespace_uri: Option<&str>,
    local_name: &str,
) -> String {
    format!(
        "__namespace:{}\u{0}{}",
        namespace_uri.unwrap_or_default(),
        local_name
    )
}

pub(in crate::native_bridge::document) fn set_attr_cache_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    key: &str,
    attr: v8::Local<'s, v8::Object>,
) {
    let _ = cache.define_own_property(
        scope,
        v8_string(scope, key)
            .map(Into::<v8::Local<'_, v8::Name>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
        attr.into(),
        v8::PropertyAttribute::NONE,
    );
}

pub(in crate::native_bridge) fn clear_live_attr_cache_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) {
    let Some(cache) = get_private_value(scope, element, ATTR_OBJECT_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    clear_cached_attrs_matching_name(scope, cache, name);
}

pub(in crate::native_bridge) fn clear_live_attr_cache_entry_ns<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace_uri: Option<&str>,
    local_name: &str,
) {
    let Some(cache) = get_private_value(scope, element, ATTR_OBJECT_CACHE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    clear_cached_attrs_matching_namespace(scope, cache, namespace_uri, local_name);
}

fn cached_attr_property_names<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    let Some(names) = cache.get_own_property_names(scope, v8::GetPropertyNamesArgs::default())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for index in 0..names.length() {
        let Some(name) = names
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
        else {
            continue;
        };
        out.push(name);
    }
    out
}

fn cached_attr_object_for_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    property: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    object_property_as_object(scope, cache, property)
}

fn state_namespace_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    namespace_uri: Option<&str>,
) -> bool {
    let value = state
        .get(scope, v8str(scope, "namespaceURI").into())
        .unwrap_or_else(|| v8::null(scope).into());
    let actual = if value.is_null_or_undefined() {
        None
    } else {
        value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
    };
    actual.as_deref() == namespace_uri.filter(|namespace| !namespace.is_empty())
}

fn cached_attr_by_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    for property in cached_attr_property_names(scope, cache) {
        let Some(attr) = cached_attr_object_for_property(scope, cache, &property) else {
            continue;
        };
        let Some(state) = attr_state_object(scope, attr) else {
            continue;
        };
        if object_string_property(scope, state, "name").as_deref() == Some(name) {
            return Some(attr);
        }
    }
    None
}

fn cached_attr_by_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    namespace_uri: Option<&str>,
    local_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    for property in cached_attr_property_names(scope, cache) {
        let Some(attr) = cached_attr_object_for_property(scope, cache, &property) else {
            continue;
        };
        let Some(state) = attr_state_object(scope, attr) else {
            continue;
        };
        if object_string_property(scope, state, "localName").as_deref() == Some(local_name)
            && state_namespace_matches(scope, state, namespace_uri)
        {
            return Some(attr);
        }
    }
    None
}

fn detach_cached_attr<'s>(scope: &mut v8::PinScope<'s, '_>, attr: v8::Local<'s, v8::Object>) {
    let current_value = attr_current_value(scope, attr);
    let Some(state) = attr_state_object(scope, attr) else {
        return;
    };
    let _ = state.set(
        scope,
        v8str(scope, "value").into(),
        v8_string(scope, &current_value)
            .map(Into::<v8::Local<'_, v8::Value>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
    );
    let _ = state.set(
        scope,
        v8str(scope, "ownerElement").into(),
        v8::null(scope).into(),
    );
}

fn clear_cache_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    property: &str,
) {
    let _ = cache.define_own_property(
        scope,
        v8_string(scope, property)
            .map(Into::<v8::Local<'_, v8::Name>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into()),
        v8::undefined(scope).into(),
        v8::PropertyAttribute::NONE,
    );
}

fn clear_cached_attr_aliases<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    attr: v8::Local<'s, v8::Object>,
) {
    let attr_value = v8::Local::<v8::Value>::from(attr);
    for property in cached_attr_property_names(scope, cache) {
        let Some(cached_attr) = cached_attr_object_for_property(scope, cache, &property) else {
            continue;
        };
        if v8::Local::<v8::Value>::from(cached_attr).strict_equals(attr_value) {
            clear_cache_property(scope, cache, &property);
        }
    }
}

fn clear_cached_attrs_matching_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    name: &str,
) {
    if let Some(attr) = object_property_as_object(scope, cache, name)
        .or_else(|| cached_attr_by_name(scope, cache, name))
    {
        detach_cached_attr(scope, attr);
        clear_cached_attr_aliases(scope, cache, attr);
    }
    clear_cache_property(scope, cache, name);
}

fn clear_cached_attrs_matching_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cache: v8::Local<'s, v8::Object>,
    namespace_uri: Option<&str>,
    local_name: &str,
) {
    let namespace_key = namespace_attr_cache_key(namespace_uri, local_name);
    if let Some(attr) = object_property_as_object(scope, cache, &namespace_key)
        .or_else(|| cached_attr_by_namespace(scope, cache, namespace_uri, local_name))
    {
        detach_cached_attr(scope, attr);
        clear_cached_attr_aliases(scope, cache, attr);
    }
    clear_cache_property(scope, cache, &namespace_key);
}

pub(crate) fn live_get_attribute_node_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    native_attr_object_by_name(scope, element, name)
}

pub(in crate::native_bridge) fn live_get_attribute_node_ns_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    namespace_uri: Option<&str>,
    local_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    native_attr_object_by_namespace(scope, element, namespace_uri, local_name)
}
