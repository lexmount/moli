use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct DetachedNamespaceAttributeRecordDeclaration<'scope> {
    name: v8::Local<'scope, v8::String>,
    value: v8::Local<'scope, v8::String>,
    #[webapi(data_property = "namespaceURI")]
    namespace_uri: v8::Local<'scope, v8::Value>,
    prefix: v8::Local<'scope, v8::Value>,
    local_name: v8::Local<'scope, v8::String>,
}

pub(in crate::native_bridge::document) fn detached_attributes_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Map>> {
    detached_state_object(scope, node)
        .and_then(|state| state.get(scope, v8str(scope, "attributes").into()))
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
}

pub(in crate::native_bridge::document) fn detached_namespace_attributes_map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Map>> {
    detached_state_object(scope, node)
        .and_then(|state| state.get(scope, v8str(scope, "namespaceAttributes").into()))
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
}

pub(in crate::native_bridge::document) fn detached_attribute_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    name: &str,
) -> String {
    let name = name.to_owned();
    let is_html_element_in_html_document =
        detached_state_string(scope, node, "documentKind").as_deref() == Some("html")
            && detached_element_namespace_uri(scope, node).as_deref() == Some(XHTML_NS);
    if is_html_element_in_html_document {
        name.to_ascii_lowercase()
    } else {
        name
    }
}

pub(in crate::native_bridge::document) fn detached_map_get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8_string(scope, key)?;
    map.get(scope, key.into())
}

pub(in crate::native_bridge::document) fn detached_map_has<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
    key: &str,
) -> bool {
    let Some(key) = v8_string(scope, key) else {
        return false;
    };
    map.has(scope, key.into()).unwrap_or(false)
}

pub(in crate::native_bridge::document) fn detached_map_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
    key: &str,
    value: &str,
) {
    let Some(key) = v8_string(scope, key) else {
        return;
    };
    let Some(value) = v8_string(scope, value) else {
        return;
    };
    let _ = map.set(scope, key.into(), value.into());
}

pub(in crate::native_bridge::document) fn detached_map_set_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(key) = v8_string(scope, key) else {
        return;
    };
    let _ = map.set(scope, key.into(), value);
}

pub(in crate::native_bridge::document) fn detached_map_set_namespace_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
    name: &str,
    value: &str,
    namespace_uri: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
) {
    let Some(record) =
        detached_namespace_attribute_record(scope, name, value, namespace_uri, prefix, local_name)
    else {
        return;
    };
    let key = namespace_attr_cache_key(namespace_uri, local_name);
    detached_map_set_value(scope, map, &key, record.into());
}

pub(in crate::native_bridge::document) fn detached_element_set_namespace_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    map: v8::Local<'s, v8::Map>,
    name: &str,
    value: &str,
    namespace_uri: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
) {
    detached_map_set_namespace_attribute(
        scope,
        map,
        name,
        value,
        namespace_uri,
        prefix,
        local_name,
    );
    sync_detached_native_set_attribute_ns(scope, element, namespace_uri, prefix, local_name, value);
    if let Some(cache) = live_attr_cache_object(scope, element)
        && let Some(attr) = new_attr_object(
            scope,
            name,
            value,
            Some(element),
            None,
            namespace_uri,
            prefix,
            local_name,
        )
    {
        let key = namespace_attr_cache_key(namespace_uri, local_name);
        set_attr_cache_entry(scope, cache, &key, attr);
    }
}

pub(in crate::native_bridge::document) fn detached_element_copy_live_namespace_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    lookup_name: v8::Local<'s, v8::String>,
    target: v8::Local<'s, v8::Object>,
    target_namespace_attributes: v8::Local<'s, v8::Map>,
) {
    let Some(attr) = call_object_method(scope, source, "getAttributeNode", &[lookup_name.into()])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let qualified_name = lookup_name.to_rust_string_lossy(scope);
    let fallback_parts = qualified_name
        .split_once(':')
        .map(|(prefix, local_name)| (prefix.to_owned(), local_name.to_owned()));
    let attr_name = object_string_property(scope, attr, "name")
        .filter(|attr_name| attr_name.contains(':') || fallback_parts.is_none())
        .unwrap_or_else(|| qualified_name.clone());
    let value = object_string_property(scope, attr, "value").unwrap_or_default();
    let local_name = object_string_property(scope, attr, "localName")
        .filter(|local_name| !local_name.is_empty())
        .or_else(|| {
            fallback_parts
                .as_ref()
                .map(|(_, local_name)| local_name.clone())
        })
        .unwrap_or_else(|| attr_name.clone());
    let namespace_uri = object_string_property(scope, attr, "namespaceURI")
        .filter(|namespace_uri| !namespace_uri.is_empty());
    let prefix = object_string_property(scope, attr, "prefix")
        .filter(|prefix| !prefix.is_empty())
        .or_else(|| fallback_parts.as_ref().map(|(prefix, _)| prefix.clone()));
    detached_element_set_namespace_attribute(
        scope,
        target,
        target_namespace_attributes,
        &attr_name,
        &value,
        namespace_uri.as_deref(),
        prefix.as_deref(),
        &local_name,
    );
}

fn detached_namespace_attribute_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    value: &str,
    namespace_uri: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let namespace_value = namespace_uri
        .and_then(|namespace_uri| v8_string(scope, namespace_uri))
        .map(Into::<v8::Local<'_, v8::Value>>::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let prefix_value = prefix
        .and_then(|prefix| v8_string(scope, prefix))
        .map(Into::<v8::Local<'_, v8::Value>>::into)
        .unwrap_or_else(|| v8::null(scope).into());
    DetachedNamespaceAttributeRecordDeclaration::new(
        v8_string(scope, name)?,
        v8_string(scope, value)?,
        namespace_value,
        prefix_value,
        v8_string(scope, local_name)?,
    )
    .bind(scope)
    .ok()
}

pub(in crate::native_bridge::document) fn detached_map_delete<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
    key: &str,
) {
    let Some(key) = v8_string(scope, key) else {
        return;
    };
    let _ = map.delete(scope, key.into());
}

pub(in crate::native_bridge::document) fn detached_map_keys_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
) -> Option<v8::Local<'s, v8::Array>> {
    let entries = map.as_array(scope);
    let keys = (0..entries.length())
        .step_by(2)
        .map(|index| entries.get_index(scope, index))
        .collect::<Option<Vec<_>>>()?;
    Some(v8::Array::new_with_elements(scope, &keys))
}
