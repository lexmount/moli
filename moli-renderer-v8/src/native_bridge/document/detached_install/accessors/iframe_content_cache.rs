use crate::{
    document_runtime::DomHandle,
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value},
};

use super::super::super::{detached_native_handle_for_runtime, detached_native_object_for_handle};

pub(super) const DETACHED_IFRAME_CONTENT_DOCUMENT_SLOT: &str = "__lmDetachedIframeContentDocument";
pub(super) const DETACHED_IFRAME_BASE_URL_SLOT: &str = "__lmDetachedIframeBaseUrl";
pub(super) const DETACHED_IFRAME_CONTENT_WINDOW_SLOT: &str = "__lmDetachedIframeContentWindow";

pub(crate) fn detached_iframe_current_content_document_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    iframe_handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(document) =
        unsafe { &*runtime_ptr }.cached_detached_iframe_content_document(scope, iframe_handle)
    {
        return detached_native_handle_for_runtime(scope, runtime_ptr, document);
    }
    let iframe = detached_native_object_for_handle(scope, runtime_ptr, iframe_handle)?;
    let document = get_private_value(scope, iframe, DETACHED_IFRAME_CONTENT_DOCUMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    detached_native_handle_for_runtime(scope, runtime_ptr, document)
}

pub(in crate::native_bridge) fn clear_detached_iframe_cached_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) {
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(iframe_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, iframe)
        && let Some(document_handle) =
            detached_iframe_current_content_document_handle(scope, runtime_ptr, iframe_handle)
    {
        let effects = [
            crate::style_engine::StyleMutationEffect::DisconnectedSubtrees {
                roots: vec![document_handle].into(),
            },
        ];
        unsafe { &mut *runtime_ptr }.note_style_mutation_effects(&effects);
    }
    clear_cached_detached_iframe_content_surfaces(scope, iframe);
    let undefined = v8::undefined(scope);
    let cache_object = detached_iframe_cache_object(scope, iframe);
    for target in [Some(iframe), cache_object].into_iter().flatten() {
        for slot in [
            DETACHED_IFRAME_CONTENT_DOCUMENT_SLOT,
            DETACHED_IFRAME_BASE_URL_SLOT,
            DETACHED_IFRAME_CONTENT_WINDOW_SLOT,
        ] {
            set_private_value(scope, target, slot, undefined.into());
        }
    }
}

pub(in crate::native_bridge) fn clear_detached_iframe_cached_context_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    iframe_handle: DomHandle,
) {
    if let Some(iframe) = detached_native_object_for_handle(scope, runtime_ptr, iframe_handle) {
        clear_detached_iframe_cached_context(scope, iframe);
    }
}

pub(super) fn detached_iframe_cache_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, iframe)?;
    unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)
}

pub(super) fn detached_iframe_runtime_and_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) -> Option<(*mut crate::native_bridge::JsContextHost, DomHandle)> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, iframe)?;
    Some((runtime_ptr, handle))
}

fn clear_cached_detached_iframe_content_surfaces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) {
    if let Some((runtime_ptr, handle)) = detached_iframe_runtime_and_handle(scope, iframe) {
        unsafe { &mut *runtime_ptr }.clear_cached_detached_iframe_content_surfaces(handle);
    }
}
