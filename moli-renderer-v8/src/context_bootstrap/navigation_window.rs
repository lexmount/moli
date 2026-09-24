use super::location_history_storage::WINDOW_CHILD_CONTEXT_HANDLE_SLOT;
use super::*;
use crate::util::{get_private_value, set_private_value};

const WINDOW_UNLOAD_EVENT_ACTIVE_SLOT: &str = "__lmWindowUnloadEventActive";
const NAVIGATION_DOCUMENT_SLOT: &str = "__lmNavigationDocument";
const NAVIGATION_LOCAL_WINDOW_ID_SLOT: &str = "__lmNavigationLocalWindowId";

pub(super) fn bind_navigation_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Object>,
) {
    if let Some(document) = navigation_owner_document(scope, owner) {
        let value = v8::BigInt::new_from_u64(scope, document.index() as u64);
        set_private_value(scope, navigation, NAVIGATION_DOCUMENT_SLOT, value.into());
    }
    if let Some(local_window_id) = navigation_owner_local_window_id(scope, owner) {
        let value = v8::BigInt::new_from_u64(scope, local_window_id);
        set_private_value(
            scope,
            navigation,
            NAVIGATION_LOCAL_WINDOW_ID_SLOT,
            value.into(),
        );
    }
}

fn navigation_owner_local_window_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let dispatch_scope = runtime_window_dispatch_scope(scope, owner)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let owner = unsafe { &*host_ptr }.current_window_execution_context_owner(dispatch_scope)?;
    Some(match owner {
        crate::native_bridge::WindowExecutionContextOwner::Frame(local_window_id) => {
            local_window_id.0
        }
        crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
            local_window_id,
            ..
        } => local_window_id.as_u64(),
    })
}

pub(super) fn navigation_belongs_to_current_local_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(local_window_id) =
        get_private_value(scope, navigation, NAVIGATION_LOCAL_WINDOW_ID_SLOT)
            .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
            .map(|value| value.u64_value().0)
    else {
        return navigation_has_current_document(scope, navigation);
    };
    let owner = runtime_window_owner(scope, navigation);
    navigation_owner_local_window_id(scope, owner) == Some(local_window_id)
}

fn navigation_owner_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<crate::document_runtime::DomHandle> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    super::window_accessors::window_document_handle(scope, owner, unsafe { &*host_ptr })
}

pub(super) fn navigation_has_current_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(document) = get_private_value(scope, navigation, NAVIGATION_DOCUMENT_SLOT)
        .and_then(|value| dom_handle_from_marker_value(scope, value))
    else {
        // Bootstrap may create the surface before registering the child realm.
        return true;
    };
    let owner = runtime_window_owner(scope, navigation);
    navigation_owner_document(scope, owner) == Some(document)
}

pub(super) fn navigation_has_disabled_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let owner = runtime_window_owner(scope, navigation);
    !navigation_has_current_document(scope, navigation)
        || navigation_document_has_disabled_entries(scope, owner)
}

pub(super) fn runtime_window_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let object = moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object);
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
    let object = moli_webapi_declare::web_api_object_target(scope, object).unwrap_or(object);
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
        Some(
            crate::native_bridge::OwnerDispatchScope::Child(_)
            | crate::native_bridge::OwnerDispatchScope::LightweightPopup(_),
        ) => false,
        None => window.strict_equals(scope.get_current_context().global(scope).into()),
    }
}

pub(super) fn runtime_window_uses_top_level_history_model<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    runtime_window_is_global(scope, window)
        || crate::native_bridge::lightweight_popup_id_from_window(scope, window).is_some()
}

pub(super) fn runtime_top_window_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    object_own_hidden_value(scope, window, WINDOW_TOP_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or_else(|| runtime_window_owner(scope, window))
}

pub(crate) fn window_location_for_holder<'s>(
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

pub(in crate::context_bootstrap) fn runtime_window_dispatch_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, window) {
        return Some(crate::native_bridge::OwnerDispatchScope::LightweightPopup(
            popup_id,
        ));
    }
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

pub(super) fn navigation_can_update_current_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation: v8::Local<'s, v8::Object>,
) -> bool {
    let owner = runtime_window_owner(scope, navigation);
    navigation_document_is_live(scope, owner) && !navigation_has_disabled_entries(scope, navigation)
}

pub(super) fn navigation_document_is_initial_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, owner) {
        return host.lightweight_popup_current_document_is_initial_empty(popup_id);
    }
    // Initialness belongs to the Document, not its URL: document.open() can
    // change that URL, and a later navigation can create a non-initial blank.
    child_browsing_context_handle_for_runtime_owner(scope, owner)
        .is_some_and(|handle| host.child_current_document_is_initial_empty(handle))
}

pub(super) fn navigation_document_has_disabled_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    if navigation_document_has_opaque_origin(scope, owner) {
        return true;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    // document.open() ends initial about:blank Window reuse, but the existing
    // Navigation object stays uninitialized until a navigation commits.
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, owner) {
        return !host.lightweight_popup_has_committed_navigation(popup_id);
    }
    child_browsing_context_handle_for_runtime_owner(scope, owner)
        .is_some_and(|handle| !host.child_has_committed_navigation(handle))
}

pub(super) fn navigation_document_has_opaque_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, owner) {
        return host
            .lightweight_popup_origin(popup_id)
            .is_some_and(|origin| origin == "null");
    }
    if runtime_window_is_global(scope, owner) {
        return top_level_navigation_document_has_opaque_origin(host.document_url());
    }
    let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, owner) else {
        return false;
    };
    host.child_browsing_context_has_opaque_origin(handle)
}

pub(crate) fn navigation_unload_event_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, owner, WINDOW_UNLOAD_EVENT_ACTIVE_SLOT)
        .is_some_and(|value| value.is_true())
        || context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
            let host = unsafe { &*host_ptr };
            super::window_accessors::window_document_handle(scope, owner, host)
                .is_some_and(|document| host.has_document_unload_counter(document))
        })
}

pub(crate) fn replace_navigation_unload_event_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    active: bool,
) -> bool {
    // Restore only the event flag, not the effective state that also includes
    // native ancestor guards. Those counters unwind independently.
    let previous = get_private_value(scope, owner, WINDOW_UNLOAD_EVENT_ACTIVE_SLOT)
        .is_some_and(|value| value.is_true());
    set_private_value(
        scope,
        owner,
        WINDOW_UNLOAD_EVENT_ACTIVE_SLOT,
        v8::Boolean::new(scope, active).into(),
    );
    previous
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
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, owner) {
        return host.lightweight_popup_request_base_url(scope, popup_id);
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
    if let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, owner) {
        return context_host_ptr_from_global_bridge(scope)
            .is_some_and(|host_ptr| unsafe { &*host_ptr }.lightweight_popup_is_open(popup_id));
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
