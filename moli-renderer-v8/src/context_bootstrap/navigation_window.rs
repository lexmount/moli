use super::location_history_storage::WINDOW_CHILD_CONTEXT_HANDLE_SLOT;
use super::*;
use crate::util::{get_private_value, set_private_value};

const WINDOW_UNLOAD_EVENT_ACTIVE_SLOT: &str = "__lmWindowUnloadEventActive";

pub(super) fn runtime_window_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_value(
        scope,
        object,
        super::location_history_storage::WINDOW_RUNTIME_OWNER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    .unwrap_or_else(|| scope.get_current_context().global(scope))
}

pub(super) fn set_runtime_window_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        object,
        super::location_history_storage::WINDOW_RUNTIME_OWNER_SLOT,
        owner.into(),
    );
}

pub(super) fn runtime_window_is_global<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    match runtime_window_dispatch_scope(scope, window) {
        Some(crate::native_bridge::OwnerDispatchScope::Top) => true,
        Some(crate::native_bridge::OwnerDispatchScope::Child(_)) => false,
        None => window.strict_equals(scope.get_current_context().global(scope).into()),
    }
}

pub(super) fn navigation_document_is_initial_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    runtime_window_is_global(scope, owner)
        && context_host_ptr_from_global_bridge(scope)
            .is_some_and(|host_ptr| unsafe { &*host_ptr }.root_document_is_initial_empty())
}

pub(super) fn runtime_window_uses_top_level_history_model<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    runtime_window_is_global(scope, window)
}

pub(super) fn runtime_top_window_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    object_own_hidden_value(scope, window, WINDOW_TOP_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or_else(|| runtime_window_owner(scope, window))
}

pub(super) fn window_location_for_holder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, window, WINDOW_LOCATION_SLOT)
        .filter(|value| !value.is_undefined())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn window_history_for_holder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    window_runtime_slot_value(scope, window, WINDOW_HISTORY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn window_navigation_for_holder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    window_runtime_slot_value(scope, window, WINDOW_NAVIGATION_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn window_runtime_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, window, slot).filter(|value| !value.is_undefined())
}

pub(in crate::context_bootstrap) fn child_browsing_context_handle_for_runtime_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<crate::document_runtime::DomHandle> {
    get_private_value(scope, owner, WINDOW_CHILD_CONTEXT_HANDLE_SLOT)
        .and_then(|value| dom_handle_from_marker_value(scope, value))
        .or_else(|| match runtime_window_dispatch_scope(scope, owner) {
            Some(crate::native_bridge::OwnerDispatchScope::Child(handle)) => Some(handle),
            _ => None,
        })
}

pub(crate) fn runtime_window_dispatch_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    if let Some(handle) = get_private_value(scope, window, WINDOW_CHILD_CONTEXT_HANDLE_SLOT)
        .and_then(|value| dom_handle_from_marker_value(scope, value))
    {
        return Some(crate::native_bridge::OwnerDispatchScope::Child(handle));
    }
    let context = window.get_creation_context(scope)?;
    let host_ptr = crate::util::context_host_ptr_from_context_slot(context)?;
    unsafe { &*host_ptr }
        .window_execution_context_identity_for_access_check(context)
        .map(|identity| identity.dispatch_scope())
}

pub(super) fn window_task_target_for_runtime_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    owner: v8::Local<'s, v8::Object>,
) -> Option<crate::native_bridge::WindowTaskTarget> {
    let dispatch_scope = runtime_window_dispatch_scope(scope, owner)?;
    let execution_owner = host.current_window_execution_context_owner(dispatch_scope)?;
    Some(crate::native_bridge::WindowTaskTarget::new(
        dispatch_scope,
        execution_owner,
    ))
}

pub(super) fn navigation_document_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    navigation_document_is_live(scope, owner)
}

pub(super) fn navigation_document_can_update_current_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    if !navigation_document_is_live(scope, owner) {
        return false;
    }
    if runtime_window_is_global(scope, owner) {
        return true;
    }
    let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, owner) else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    unsafe { &*host_ptr }
        .child_browsing_context_current_url(handle)
        .is_some_and(|url| !url_is_about_blank_document(&url))
}

pub(super) fn navigation_document_has_opaque_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    if runtime_window_is_global(scope, owner) {
        return host
            .document_policy_container()
            .sandbox
            .forces_opaque_origin
            || top_level_navigation_document_has_opaque_origin(host.document_url());
    }
    let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, owner) else {
        return false;
    };
    host.child_browsing_context_has_opaque_origin(handle)
}

pub(super) fn navigation_unload_event_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    object_bool_property(scope, owner, WINDOW_UNLOAD_EVENT_ACTIVE_SLOT).unwrap_or(false)
}

pub(super) fn set_navigation_unload_event_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    active: bool,
) {
    define_non_enumerable_bool_property(scope, owner, WINDOW_UNLOAD_EVENT_ACTIVE_SLOT, active);
}

pub(super) fn url_is_about_blank_document(url: &url::Url) -> bool {
    moli_url::is_about_blank(url)
}

fn top_level_navigation_document_has_opaque_origin(url: &url::Url) -> bool {
    !url_is_about_blank_document(url) && url.origin().ascii_serialization() == "null"
}

pub(super) fn navigation_document_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    fallback_href: &str,
) -> Option<url::Url> {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return url::Url::parse(fallback_href).ok();
    };
    let host = unsafe { &*host_ptr };
    if runtime_window_is_global(scope, owner) {
        return Some(
            host.dom_host()
                .document_base_url()
                .unwrap_or_else(|| host.document_url().clone()),
        );
    }
    let handle = child_browsing_context_handle_for_runtime_owner(scope, owner)?;
    let base = host
        .child_browsing_context_base_url(handle)
        .or_else(|| url::Url::parse(fallback_href).ok())?;
    if base.as_str() == "about:blank" {
        return Some(
            host.dom_host()
                .document_base_url()
                .unwrap_or_else(|| host.document_url().clone()),
        );
    }
    Some(base)
}

fn navigation_document_is_live<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    if runtime_window_is_global(scope, owner) {
        return true;
    }
    let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, owner) else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    if !host.child_browsing_context_is_live(handle) {
        return false;
    }
    true
}

pub(in crate::context_bootstrap) fn dom_handle_from_marker_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<crate::document_runtime::DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless.then(|| crate::document_runtime::DomHandle::new(index as usize));
    }
    let value = value.number_value(scope)?;
    (value.is_finite() && value >= 0.0 && value.fract() == 0.0)
        .then(|| crate::document_runtime::DomHandle::new(value as usize))
}

pub(super) fn should_dispatch_hash_change(old_url: &str, new_url: &str) -> bool {
    if old_url == new_url {
        return false;
    }
    let Ok(old) = url::Url::parse(old_url) else {
        return false;
    };
    let Ok(new) = url::Url::parse(new_url) else {
        return false;
    };
    super::location_runtime::is_same_document_fragment_navigation(Some(&old), &new)
        && old.fragment() != new.fragment()
}
