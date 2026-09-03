use super::*;
use crate::custom_elements;
use crate::util::context_host_ptr_from_global_bridge;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct DetachedSurfaceNamespaceAttributeRecordDeclaration<'scope> {
    name: v8::Local<'scope, v8::String>,
    value: v8::Local<'scope, v8::String>,
    #[webapi(data_property = "namespaceURI")]
    namespace_uri: v8::Local<'scope, v8::Value>,
    prefix: v8::Local<'scope, v8::Value>,
    local_name: v8::Local<'scope, v8::String>,
}

fn required_callback_arg_string(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    index: i32,
    method: &str,
) -> Option<String> {
    if args.length() <= index {
        throw_type_error(
            scope,
            &format!("Failed to execute '{method}' on 'Element': 1 argument required."),
        );
        return None;
    }
    args.get(index)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(in crate::native_bridge) fn bridge_detached_get_attribute_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(name) = required_callback_arg_string(scope, &args, 1, "getAttribute") else {
        rv.set_null();
        return;
    };
    let normalized = detached_attribute_name(scope, element, &name);
    if let Some(has_attribute) = read_detached_native_has_attribute(scope, element, &normalized) {
        if has_attribute
            && let Some(value) = read_detached_native_attribute(scope, element, &normalized)
        {
            set_string_return_value(scope, &mut rv, &value);
        } else {
            rv.set_null();
        }
        return;
    }
    let Some(attributes) = detached_attributes_map(scope, element) else {
        rv.set_null();
        return;
    };
    match detached_map_get(scope, attributes, &normalized) {
        Some(value) if !value.is_undefined() => rv.set(value),
        _ => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_get_attribute_ns_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let namespace = callback_arg_namespace(scope, &args, 1);
    let Some(local_name) = required_callback_arg_string(scope, &args, 2, "getAttributeNS") else {
        rv.set_null();
        return;
    };
    if let Some(has_attribute) =
        read_detached_native_has_attribute_ns(scope, element, namespace.as_deref(), &local_name)
    {
        if has_attribute
            && let Some(value) =
                read_detached_native_attribute_ns(scope, element, namespace.as_deref(), &local_name)
        {
            set_string_return_value(scope, &mut rv, &value);
        } else {
            rv.set_null();
        }
        return;
    }
    let Some(namespace_attributes) = detached_namespace_attributes_map(scope, element) else {
        rv.set_null();
        return;
    };
    let key = namespace_attr_cache_key(namespace.as_deref(), &local_name);
    let Some(record) = detached_map_get(scope, namespace_attributes, &key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_null();
        return;
    };
    match record.get(scope, v8str(scope, "value").into()) {
        Some(value) if !value.is_undefined() => rv.set(value),
        _ => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn bridge_detached_get_attribute_names_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    if let Some(names) = read_detached_native_attribute_names(scope, element) {
        rv.set(string_array(scope, names).into());
        return;
    }
    let Some(attributes) = detached_attributes_map(scope, element) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    match detached_map_keys_array(scope, attributes) {
        Some(keys) => rv.set(keys.into()),
        None => rv.set(v8::Array::new(scope, 0).into()),
    }
}

pub(in crate::native_bridge) fn bridge_detached_has_attribute_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let Some(name) = required_callback_arg_string(scope, &args, 1, "hasAttribute") else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let normalized = detached_attribute_name(scope, element, &name);
    if let Some(has_attribute) = read_detached_native_has_attribute(scope, element, &normalized) {
        rv.set(v8::Boolean::new(scope, has_attribute).into());
        return;
    }
    let Some(attributes) = detached_attributes_map(scope, element) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let has_attribute = detached_map_has(scope, attributes, &normalized);
    rv.set(v8::Boolean::new(scope, has_attribute).into());
}

pub(in crate::native_bridge) fn bridge_detached_has_attribute_ns_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let namespace = callback_arg_namespace(scope, &args, 1);
    let Some(local_name) = required_callback_arg_string(scope, &args, 2, "hasAttributeNS") else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    if let Some(present) =
        read_detached_native_has_attribute_ns(scope, element, namespace.as_deref(), &local_name)
    {
        rv.set(v8::Boolean::new(scope, present).into());
        return;
    }
    let present =
        detached_namespace_attributes_map(scope, element).is_some_and(|namespace_attributes| {
            detached_map_has(
                scope,
                namespace_attributes,
                &namespace_attr_cache_key(namespace.as_deref(), &local_name),
            )
        });
    rv.set(v8::Boolean::new(scope, present).into());
}

fn string_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: Vec<String>,
) -> v8::Local<'s, v8::Array> {
    let values = values
        .iter()
        .map(|value| {
            v8_string(scope, value)
                .map(Into::<v8::Local<'s, v8::Value>>::into)
                .unwrap_or_else(|| v8::String::empty(scope).into())
        })
        .collect::<Vec<_>>();
    v8::Array::new_with_elements(scope, &values)
}

pub(in crate::native_bridge) fn bridge_detached_set_attribute_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let Some(name) = required_callback_arg_string(scope, &args, 1, "setAttribute") else {
        return;
    };
    let normalized = detached_attribute_name(scope, element, &name);
    if !validate_attribute_name(&normalized) {
        throw_dom_exception(
            scope,
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        );
        return;
    }
    if args.length() <= 2 {
        throw_type_error(
            scope,
            "Failed to execute 'setAttribute' on 'Element': 2 arguments required.",
        );
        return;
    }
    let runtime_and_handle = detached_surface_runtime_and_handle(scope, args.this(), element);
    let Some(value) = super::super::super::element::trusted_attribute_value_string(
        scope,
        runtime_and_handle,
        None,
        &normalized,
        args.get(2),
        super::super::super::element::TrustedAttributeSetter::SetAttribute,
    ) else {
        return;
    };
    with_detached_surface_attribute_reaction_scope(scope, args.this(), element, |scope| {
        if let Some(has_native_attribute) =
            read_detached_native_has_attribute(scope, element, &normalized)
        {
            if has_native_attribute {
                clear_matching_namespace_attr_cache_by_name(scope, element, &normalized);
            }
            if sync_detached_surface_set_attribute(scope, args.this(), element, &normalized, &value)
                .unwrap_or(false)
            {
                detached_record_tree_mutation(scope, element);
            }
            return;
        }
        let Some(attributes) = detached_attributes_map(scope, element) else {
            return;
        };
        detached_remove_namespace_attribute_by_name(scope, args.this(), element, &normalized);
        detached_map_set(scope, attributes, &normalized, &value);
        let _ =
            sync_detached_surface_set_attribute(scope, args.this(), element, &normalized, &value);
        detached_record_tree_mutation(scope, element);
    });
}

pub(in crate::native_bridge) fn bridge_detached_set_attribute_ns_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let namespace = callback_arg_namespace(scope, &args, 1);
    let Some(qualified_name) = required_callback_arg_string(scope, &args, 2, "setAttributeNS")
    else {
        return;
    };
    let (prefix, local_name) =
        match validate_qualified_name_and_namespace(namespace.as_deref(), &qualified_name) {
            Ok(parts) => parts,
            Err((name, code, message)) => {
                throw_dom_exception(scope, name, code, message);
                return;
            }
        };
    if args.length() <= 3 {
        throw_type_error(
            scope,
            "Failed to execute 'setAttributeNS' on 'Element': 3 arguments required.",
        );
        return;
    }
    let runtime_and_handle = detached_surface_runtime_and_handle(scope, args.this(), element);
    let Some(value) = super::super::super::element::trusted_attribute_value_string(
        scope,
        runtime_and_handle,
        namespace.as_deref(),
        &local_name,
        args.get(3),
        super::super::super::element::TrustedAttributeSetter::SetAttributeNs,
    ) else {
        return;
    };
    with_detached_surface_attribute_reaction_scope(scope, args.this(), element, |scope| {
        if let Some(has_native_attribute) =
            read_detached_native_has_attribute_ns(scope, element, namespace.as_deref(), &local_name)
        {
            if has_native_attribute {
                clear_live_attr_cache_entry_ns(scope, element, namespace.as_deref(), &local_name);
            }
            if sync_detached_surface_set_attribute_ns(
                scope,
                args.this(),
                element,
                namespace.as_deref(),
                prefix.as_deref(),
                &local_name,
                &qualified_name,
                &value,
            )
            .unwrap_or(false)
            {
                detached_record_tree_mutation(scope, element);
            }
            if let Some(cache) = live_attr_cache_object(scope, element)
                && let Some(attr) = new_attr_object(
                    scope,
                    &qualified_name,
                    &value,
                    Some(element),
                    None,
                    namespace.as_deref(),
                    prefix.as_deref(),
                    &local_name,
                )
            {
                set_attr_cache_entry(
                    scope,
                    cache,
                    &namespace_attr_cache_key(namespace.as_deref(), &local_name),
                    attr,
                );
            }
            return;
        }
        let Some(attributes) = detached_attributes_map(scope, element) else {
            return;
        };
        let Some(namespace_attributes) = detached_namespace_attributes_map(scope, element) else {
            return;
        };
        let key = namespace_attr_cache_key(namespace.as_deref(), &local_name);
        if let Some(old_record) = detached_map_get(scope, namespace_attributes, &key)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            && let Some(old_name) = object_string_property(scope, old_record, "name")
        {
            detached_map_delete(scope, attributes, &old_name);
        }
        detached_map_set(scope, attributes, &qualified_name, &value);
        let _ = sync_detached_surface_set_attribute_ns(
            scope,
            args.this(),
            element,
            namespace.as_deref(),
            prefix.as_deref(),
            &local_name,
            &qualified_name,
            &value,
        );

        let record = DetachedSurfaceNamespaceAttributeRecordDeclaration {
            name: v8_string(scope, &qualified_name).unwrap_or_else(|| v8::String::empty(scope)),
            value: v8_string(scope, &value).unwrap_or_else(|| v8::String::empty(scope)),
            namespace_uri: namespace
                .as_deref()
                .and_then(|namespace| v8_string(scope, namespace))
                .map(Into::<v8::Local<'_, v8::Value>>::into)
                .unwrap_or_else(|| v8::null(scope).into()),
            prefix: prefix
                .as_deref()
                .and_then(|prefix| v8_string(scope, prefix))
                .map(Into::<v8::Local<'_, v8::Value>>::into)
                .unwrap_or_else(|| v8::null(scope).into()),
            local_name: v8_string(scope, &local_name).unwrap_or_else(|| v8::String::empty(scope)),
        }
        .bind(scope)
        .expect("detached namespace attribute record declaration should bind");
        let key_value = v8_string(scope, &key)
            .map(Into::<v8::Local<'_, v8::Value>>::into)
            .unwrap_or_else(|| v8::String::empty(scope).into());
        let _ = namespace_attributes.set(scope, key_value, record.into());

        if let Some(cache) = live_attr_cache_object(scope, element)
            && let Some(attr) = new_attr_object(
                scope,
                &qualified_name,
                &value,
                Some(element),
                None,
                namespace.as_deref(),
                prefix.as_deref(),
                &local_name,
            )
        {
            set_attr_cache_entry(scope, cache, &key, attr);
        }
        detached_record_tree_mutation(scope, element);
    });
}

pub(in crate::native_bridge) fn bridge_detached_remove_attribute_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let Some(name) = required_callback_arg_string(scope, &args, 1, "removeAttribute") else {
        return;
    };
    let normalized = detached_attribute_name(scope, element, &name);
    with_detached_surface_attribute_reaction_scope(scope, args.this(), element, |scope| {
        if let Some(has_native_attribute) =
            read_detached_native_has_attribute(scope, element, &normalized)
        {
            if has_native_attribute {
                let removed = sync_detached_surface_remove_attribute(
                    scope,
                    args.this(),
                    element,
                    &normalized,
                )
                .unwrap_or(false);
                clear_live_attr_cache_entry(scope, element, &normalized);
                if removed {
                    detached_record_tree_mutation(scope, element);
                }
            }
            return;
        }
        let Some(attributes) = detached_attributes_map(scope, element) else {
            return;
        };
        detached_remove_namespace_attribute_by_name(scope, args.this(), element, &normalized);
        detached_map_delete(scope, attributes, &normalized);
        let _ = sync_detached_surface_remove_attribute(scope, args.this(), element, &normalized);
        detached_record_tree_mutation(scope, element);
    });
}

pub(in crate::native_bridge) fn bridge_detached_remove_attribute_ns_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let namespace = callback_arg_namespace(scope, &args, 1);
    let Some(local_name) = required_callback_arg_string(scope, &args, 2, "removeAttributeNS")
    else {
        return;
    };
    with_detached_surface_attribute_reaction_scope(scope, args.this(), element, |scope| {
        if let Some(has_native_attribute) =
            read_detached_native_has_attribute_ns(scope, element, namespace.as_deref(), &local_name)
        {
            if has_native_attribute {
                let removed = sync_detached_surface_remove_attribute_ns(
                    scope,
                    args.this(),
                    element,
                    namespace.as_deref(),
                    &local_name,
                )
                .unwrap_or(false);
                clear_live_attr_cache_entry_ns(scope, element, namespace.as_deref(), &local_name);
                if removed {
                    detached_record_tree_mutation(scope, element);
                }
            }
            return;
        }
        let Some(namespace_attributes) = detached_namespace_attributes_map(scope, element) else {
            return;
        };
        let key = namespace_attr_cache_key(namespace.as_deref(), &local_name);
        if let Some(attributes) = detached_attributes_map(scope, element)
            && let Some(record) = detached_map_get(scope, namespace_attributes, &key)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            && let Some(name) = object_string_property(scope, record, "name")
        {
            detached_map_delete(scope, attributes, &name);
        }
        detached_map_delete(scope, namespace_attributes, &key);
        let _ = sync_detached_surface_remove_attribute_ns(
            scope,
            args.this(),
            element,
            namespace.as_deref(),
            &local_name,
        );
        clear_live_attr_cache_entry_ns(scope, element, namespace.as_deref(), &local_name);
        detached_record_tree_mutation(scope, element);
    });
}

fn detached_surface_runtime_and_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bridge: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, element) {
        return Some((runtime_ptr, handle));
    }
    let runtime_ptr = runtime_ptr_from_object(scope, bridge)
        .ok()
        .or_else(|| context_host_ptr_from_global_bridge(scope));
    let runtime_ptr = runtime_ptr?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, element)?;
    Some((runtime_ptr, handle))
}

fn with_detached_surface_attribute_reaction_scope<'scope, 'pin, R>(
    scope: &mut v8::PinScope<'scope, 'pin>,
    bridge: v8::Local<'scope, v8::Object>,
    element: v8::Local<'scope, v8::Object>,
    op: impl FnOnce(&mut v8::PinScope<'scope, 'pin>) -> R,
) -> R {
    let Some((runtime_ptr, _)) = detached_surface_runtime_and_handle(scope, bridge, element) else {
        return op(scope);
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, op)
}

fn sync_detached_surface_set_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bridge: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) -> Option<bool> {
    let (runtime_ptr, handle) = detached_surface_runtime_and_handle(scope, bridge, element)?;
    let old_value = unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute(handle, name);
    let clears_iframe_context = old_value.as_deref() != Some(value)
        && detached_iframe_navigation_attribute_changed(runtime_ptr, handle, name);
    if clears_iframe_context {
        clear_detached_iframe_cached_context(scope, element);
    }
    Some(
        crate::native_bridge::element::set_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            name,
            value,
        ),
    )
}

fn sync_detached_surface_set_attribute_ns<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bridge: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    prefix: Option<&str>,
    local_name: &str,
    qualified_name: &str,
    value: &str,
) -> Option<bool> {
    let (runtime_ptr, handle) = detached_surface_runtime_and_handle(scope, bridge, element)?;
    Some(crate::native_bridge::element::set_live_element_attribute_ns_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        namespace,
        prefix,
        local_name,
        qualified_name,
        value,
    ))
}

fn sync_detached_surface_remove_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bridge: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<bool> {
    let (runtime_ptr, handle) = detached_surface_runtime_and_handle(scope, bridge, element)?;
    let old_value = unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute(handle, name);
    let had_attribute = old_value.is_some();
    let clears_iframe_context =
        had_attribute && detached_iframe_navigation_attribute_changed(runtime_ptr, handle, name);
    if clears_iframe_context {
        clear_detached_iframe_cached_context(scope, element);
    }
    Some(
        crate::native_bridge::element::remove_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            name,
        ),
    )
}

fn detached_iframe_navigation_attribute_changed(
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
) -> bool {
    matches!(name, "src" | "srcdoc")
        && unsafe { &*runtime_ptr }
            .dom_host()
            .is_html_element_named(handle, "iframe")
}

fn sync_detached_surface_remove_attribute_ns<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bridge: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
    namespace: Option<&str>,
    local_name: &str,
) -> Option<bool> {
    let (runtime_ptr, handle) = detached_surface_runtime_and_handle(scope, bridge, element)?;
    Some(crate::native_bridge::element::remove_live_element_attribute_ns_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        handle,
        namespace,
        local_name,
    ))
}

fn clear_matching_namespace_attr_cache_by_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) {
    let Some(attributes) = read_detached_native_attribute_snapshot(scope, element) else {
        return;
    };
    for attribute in attributes {
        if attribute.name != name {
            continue;
        }
        if attribute.namespace_uri.is_some()
            || attribute
                .prefix
                .as_deref()
                .is_some_and(|prefix| !prefix.is_empty())
        {
            clear_live_attr_cache_entry_ns(
                scope,
                element,
                attribute.namespace_uri.as_deref(),
                &attribute.local_name,
            );
        }
    }
}

fn detached_remove_namespace_attribute_by_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bridge: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
    name: &str,
) {
    let Some(namespace_attributes) = detached_namespace_attributes_map(scope, element) else {
        return;
    };
    let Some(keys) = detached_map_keys_array(scope, namespace_attributes) else {
        return;
    };
    for index in 0..keys.length() {
        let Some(key) = keys
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
        else {
            continue;
        };
        let Some(record) = detached_map_get(scope, namespace_attributes, &key)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        if object_string_property(scope, record, "name").as_deref() != Some(name) {
            continue;
        }
        let namespace = object_string_property(scope, record, "namespaceURI");
        let local_name = object_string_property(scope, record, "localName");
        detached_map_delete(scope, namespace_attributes, &key);
        if let Some(local_name) = local_name {
            let _ = sync_detached_surface_remove_attribute_ns(
                scope,
                bridge,
                element,
                namespace.as_deref(),
                &local_name,
            );
            clear_live_attr_cache_entry_ns(scope, element, namespace.as_deref(), &local_name);
        }
    }
}
