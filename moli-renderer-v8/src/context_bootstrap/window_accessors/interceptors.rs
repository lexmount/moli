use super::helpers::{
    window_child_context_handle, window_document_handle, window_host_ptr,
    window_owner_dispatch_scope,
};
use crate::native_bridge::named_access::{
    build_window_named_items_collection, window_named_item_handles,
};
use crate::util::serialize_v8_iter_array;
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

fn is_reserved_window_name(name: &str) -> bool {
    matches!(name, "window" | "self" | "top" | "parent" | "frames")
}

fn window_indexed_child_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<(
    *mut crate::native_bridge::JsContextHost,
    crate::document_runtime::DomHandle,
    crate::native_bridge::OwnerDispatchScope,
)> {
    let host_ptr = window_host_ptr(scope, receiver)?;
    let host = unsafe { &mut *host_ptr };
    let owner_scope = window_owner_dispatch_scope(scope, receiver)?;
    let document = window_document_handle(scope, receiver, host)?;
    host.sync_child_browsing_context_subtree(scope, document);
    let handle =
        host.child_browsing_context_handle_by_index_for_document(document, index as usize)?;
    Some((host_ptr, handle, owner_scope))
}

fn window_child_projection_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    owner_scope: crate::native_bridge::OwnerDispatchScope,
) -> Option<v8::Local<'s, v8::Object>> {
    if owner_scope == crate::native_bridge::OwnerDispatchScope::Top {
        host.child_browsing_context_window_proxy_for_top(scope, handle)
    } else {
        host.child_browsing_context_window_wrapper(scope, handle)
    }
}

pub(in crate::context_bootstrap) fn window_indexed_property_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some((host_ptr, handle, owner_scope)) =
        window_indexed_child_handle(scope, args.holder(), index)
    else {
        return v8::Intercepted::kNo;
    };
    let host = unsafe { &mut *host_ptr };
    let window = window_child_projection_for_owner(scope, host, handle, owner_scope);
    let Some(window) = window else {
        return v8::Intercepted::kNo;
    };
    rv.set(window.into());
    v8::Intercepted::kYes
}

pub(in crate::context_bootstrap) fn window_indexed_property_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if window_indexed_child_handle(scope, args.holder(), index).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(in crate::context_bootstrap) fn window_indexed_property_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let receiver = args.holder();
    let Some(host_ptr) = window_host_ptr(scope, receiver) else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let count = window_document_handle(scope, receiver, host)
        .map(|document| {
            host.sync_child_browsing_context_subtree(scope, document);
            host.child_browsing_context_count_for_document(document)
        })
        .unwrap_or(0);
    let array = serialize_v8_iter_array(scope, (0..count).map(|index| index as u32))
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(array);
}

pub(in crate::context_bootstrap) fn window_indexed_property_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some((host_ptr, handle, owner_scope)) =
        window_indexed_child_handle(scope, args.holder(), index)
    else {
        return v8::Intercepted::kNo;
    };
    let host = unsafe { &mut *host_ptr };
    let window = window_child_projection_for_owner(scope, host, handle, owner_scope);
    let Some(window) = window else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) =
        DataPropertyDescriptorDeclaration::new(window.into(), false, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(in crate::context_bootstrap) fn window_named_property_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = window_named_access_value(scope, key, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

pub(in crate::context_bootstrap) fn window_named_property_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if window_named_access_value(scope, key, args.holder()).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(in crate::context_bootstrap) fn window_named_properties_indexed_property_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let index = index.to_string();
    let Some(key) = crate::util::v8_string(scope, &index) else {
        return v8::Intercepted::kNo;
    };
    let key: v8::Local<'s, v8::Name> = key.into();
    let Some(value) = window_named_access_value(scope, key, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

pub(in crate::context_bootstrap) fn window_named_properties_indexed_property_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let index = index.to_string();
    let Some(key) = crate::util::v8_string(scope, &index) else {
        return v8::Intercepted::kNo;
    };
    let key: v8::Local<'s, v8::Name> = key.into();
    if window_named_access_value(scope, key, args.holder()).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
    v8::Intercepted::kYes
}

fn window_named_property_is_shadowed_by_prototype(
    scope: &mut v8::PinScope<'_, '_>,
    holder: v8::Local<'_, v8::Object>,
    key: v8::Local<'_, v8::String>,
) -> bool {
    // Web IDL's named-property visibility algorithm makes the anonymous
    // WindowProperties getter yield to own properties on later prototypes.
    let mut prototype = holder.get_prototype(scope);
    while let Some(value) = prototype {
        if value.is_null_or_undefined() {
            return false;
        }
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            return false;
        };
        if object.has_own_property(scope, key.into()).unwrap_or(false) {
            return true;
        }
        prototype = object.get_prototype(scope);
    }
    false
}

fn window_named_access_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    holder: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8::Local::<v8::String>::try_from(key).ok()?;
    if window_named_property_is_shadowed_by_prototype(scope, holder, key) {
        return None;
    }
    let key_name = key.to_rust_string_lossy(scope);
    if key_name.is_empty() {
        return None;
    }
    if is_reserved_window_name(&key_name) {
        return None;
    }
    let host_ptr = window_host_ptr(scope, holder)?;
    let child_handle = window_child_context_handle(scope, holder);
    // Browsing-context names win over document id/name exposure, matching the
    // Window named access ordering. Both paths are fast miss paths now: child
    // contexts return immediately when empty, and document name lookup is indexed.
    if let Some(handle) =
        unsafe { &*host_ptr }.child_browsing_context_named_child_handle(child_handle, &key_name)
        && let Some(window) =
            unsafe { &mut *host_ptr }.child_browsing_context_window_wrapper(scope, handle)
    {
        return Some(window.into());
    }
    let host = unsafe { &*host_ptr };
    let document = child_handle
        .and_then(|child_handle| host.child_browsing_context_document_handle(child_handle))
        .unwrap_or_else(|| host.document_handle());
    let handles = window_named_item_handles(host.dom_host(), document, &key_name);
    if moli_trace::window_message_trace_enabled() {
        tracing::info!(
            target: "moli_window_message_trace",
            property = %key_name,
            child_handle = child_handle.map(|handle| handle.index()),
            document_handle = document.index(),
            matched_handle = handles.first().map(|handle| handle.index()),
            match_count = handles.len(),
            receiver_is_current_global = holder
                .strict_equals(scope.get_current_context().global(scope).into()),
            stage = "window_named_property_lookup",
        );
    }
    match handles.as_slice() {
        [] => None,
        [handle] => unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, *handle)
            .map(Into::into),
        _ => build_window_named_items_collection(scope, host_ptr, document, &key_name)
            .map(Into::into),
    }
}
