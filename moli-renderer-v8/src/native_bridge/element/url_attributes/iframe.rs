use crate::document_runtime::DomHandle;
use crate::util::get_private_value;
use url::Url;

use super::super::super::JsContextHost;
use super::super::set_reflected_attribute;
use super::helpers::resolve_url_like_attribute;

const CHILD_DOCUMENT_CONTEXT_HANDLE_SLOT: &str = "__lmChildDocumentContextHandle";

pub(in crate::native_bridge) fn update_iframe_snapshot_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    value: &str,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    let previous_seed_snapshot = runtime.child_browsing_context_navigation_seed_snapshot(handle);
    let old_navigation_target = current_iframe_navigation_target(runtime, handle);
    let old_attribute_target = iframe_navigation_target(runtime, handle);
    set_reflected_attribute(scope, runtime_ptr, handle, "src", value);
    let runtime = unsafe { &mut *runtime_ptr };
    if !runtime.dom_host().is_html_element_named(handle, "iframe") {
        return;
    }

    let navigation_target = iframe_navigation_target(runtime, handle);
    if old_navigation_target.as_deref() == Some(navigation_target.as_str())
        && old_attribute_target == navigation_target
        && runtime.is_dispatching_child_browsing_context_host_load(handle)
    {
        return;
    }
    if let Some(previous_seed_snapshot) = previous_seed_snapshot
        && !previous_seed_snapshot.pending_attribute_bootstrap_commit
    {
        runtime.restore_child_browsing_context_navigation_seed_snapshot(
            handle,
            previous_seed_snapshot,
        );
        let replace_current =
            runtime.child_browsing_context_is_on_initial_about_blank_entry(handle);
        if runtime.queue_child_browsing_context_navigation_from_existing_seed(
            handle,
            &navigation_target,
            replace_current,
        ) {
            return;
        }
    }

    if runtime.queue_child_browsing_context_navigation_from_existing_seed(
        handle,
        &navigation_target,
        true,
    ) {
        return;
    }

    let cached_snapshot = Url::parse(&navigation_target)
        .ok()
        .and_then(|url| runtime.materialize_local_child_snapshot_for_navigation_url(handle, &url));
    runtime.cache_child_browsing_context_snapshot(scope, handle, cached_snapshot);
    runtime.sync_existing_child_browsing_context_window_state(scope, handle);
    let _ = runtime.queue_child_document_complete_lifecycle_if_ready(handle);
}

fn current_iframe_navigation_target(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    if let Some(current_url) = runtime.child_browsing_context_visible_url(handle) {
        return Some(current_url);
    }
    let resolved_src = resolve_url_like_attribute(runtime, handle, "src");
    Some(if resolved_src.is_empty() {
        "about:blank".to_owned()
    } else {
        resolved_src
    })
}

fn iframe_navigation_target(runtime: &JsContextHost, handle: DomHandle) -> String {
    let resolved_src = resolve_url_like_attribute(runtime, handle, "src");
    if resolved_src.is_empty() {
        "about:blank".to_owned()
    } else {
        resolved_src
    }
}

pub(in crate::native_bridge::element) fn disconnected_iframe_can_materialize_detached_content(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    !runtime.dom_host().is_connected(handle) && iframe_uses_detached_content_cache(runtime, handle)
}

pub(in crate::native_bridge::element) fn iframe_uses_detached_content_cache(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    if !runtime.dom_host().is_html_element_named(handle, "iframe") {
        return false;
    }
    let Some(owner_document) = runtime.dom_host().owner_document_handle(handle) else {
        return false;
    };
    if owner_document == runtime.dom_host().document_handle()
        || runtime
            .child_browsing_context_host_for_document_handle(owner_document)
            .is_some()
        || runtime
            .lightweight_popup_id_for_document_handle(owner_document)
            .is_some()
    {
        return false;
    }
    !runtime.child_browsing_context_host_is_ancestor_of_document(handle, owner_document)
}

pub(in crate::native_bridge::element) fn iframe_is_in_own_child_document(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    runtime
        .dom_host()
        .owner_document_handle(handle)
        .is_some_and(|owner_document| {
            runtime.child_browsing_context_host_is_ancestor_of_document(handle, owner_document)
        })
}

pub(in crate::native_bridge::element) fn iframe_has_inactive_child_context(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    runtime
        .child_browsing_context_document_handle(handle)
        .is_some()
        && !runtime.child_browsing_context_is_live(handle)
}

pub(in crate::native_bridge) fn live_frame_owner_content_window_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.refresh_child_browsing_context(scope, handle);
    let exposes_same_origin_wrapper =
        runtime.child_browsing_context_is_same_origin_with_top(handle);
    let window = runtime.child_browsing_context_window_proxy_for_top(scope, handle);
    if window.is_some() {
        runtime.mark_child_browsing_context_window_wrapper_exposed_to_top(handle);
    }
    if exposes_same_origin_wrapper && window.is_some() {
        runtime.request_child_frame_realm_materialization(handle);
    }
    if exposes_same_origin_wrapper && let Some(window) = window {
        runtime.set_cached_detached_iframe_content_window(scope, handle, window);
    }
    window
}

pub(in crate::native_bridge::element) fn iframe_is_inside_its_own_child_context_document(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(owner_document) = runtime.dom_host().owner_document_handle(handle) else {
        return false;
    };
    let Some(owner_document) =
        runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, owner_document)
    else {
        return false;
    };
    get_private_value(scope, owner_document, CHILD_DOCUMENT_CONTEXT_HANDLE_SLOT)
        .and_then(|value| dom_handle_from_marker_value(scope, value))
        == Some(handle)
}

fn dom_handle_from_marker_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless.then(|| DomHandle::new(index as usize));
    }
    let value = value.number_value(scope)?;
    (value.is_finite() && value >= 0.0 && value.fract() == 0.0)
        .then(|| DomHandle::new(value as usize))
}
