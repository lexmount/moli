use crate::{
    parser::HtmlParser,
    util::{get_private_value, set_private_value, v8_string, v8str},
};
use url::Url;

use super::super::super::{detached_owner_document_object, detached_state_object};
use super::attributes::detached_element_attribute_value;
use super::iframe_content_cache::{
    DETACHED_IFRAME_BASE_URL_SLOT, DETACHED_IFRAME_CONTENT_DOCUMENT_SLOT,
    DETACHED_IFRAME_CONTENT_WINDOW_SLOT, detached_iframe_cache_object,
    detached_iframe_has_cached_content_document, detached_iframe_runtime_and_handle,
    live_child_browsing_context_handle_for_detached_iframe,
};
use super::iframe_window::build_detached_iframe_content_window;
use super::url_helpers::resolve_detached_url_attribute;

pub(in crate::native_bridge) fn detached_iframe_content_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache_object = detached_iframe_cache_object(scope, iframe).unwrap_or(iframe);
    if let Some((runtime_ptr, handle)) = detached_iframe_runtime_and_handle(scope, iframe)
        && let Some(cached) =
            unsafe { &*runtime_ptr }.cached_detached_iframe_content_document(scope, handle)
    {
        return Some(cached);
    }
    if let Some(cached) =
        get_private_value(scope, cache_object, DETACHED_IFRAME_CONTENT_DOCUMENT_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(cached);
    }
    if let Some((runtime_ptr, handle)) =
        live_child_browsing_context_handle_for_detached_iframe(scope, iframe)
    {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime.child_browsing_context_window_wrapper(scope, handle)?;
        return runtime.child_browsing_context_document_wrapper(scope, handle);
    }
    let snapshot = detached_iframe_snapshot(scope, iframe)?;
    let parsed_base_url = snapshot_base_url(snapshot.url.clone(), &snapshot.markup);
    let base_url =
        if moli_url::is_about_blank(&snapshot.url) && moli_url::is_about_blank(&parsed_base_url) {
            detached_iframe_srcdoc_base_url(scope, iframe)?
        } else {
            parsed_base_url
        };
    let document = crate::dom_parser::parse_browsing_context_document_projection_from_source(
        scope,
        snapshot.url.clone(),
        &snapshot.markup,
        snapshot.content_type.as_deref(),
        Some(&snapshot.character_set),
        HtmlParser::with_scripting_enabled(false),
    )?;
    if let Some(base_url) = v8_string(scope, base_url.as_str()) {
        set_private_value(
            scope,
            cache_object,
            DETACHED_IFRAME_BASE_URL_SLOT,
            base_url.into(),
        );
        if let Some(state) = detached_state_object(scope, document) {
            let _ = state.set(scope, v8str(scope, "baseURI").into(), base_url.into());
        }
    }
    set_private_value(
        scope,
        cache_object,
        DETACHED_IFRAME_CONTENT_DOCUMENT_SLOT,
        document.into(),
    );
    if let Some((runtime_ptr, handle)) = detached_iframe_runtime_and_handle(scope, iframe) {
        unsafe { &mut *runtime_ptr }
            .set_cached_detached_iframe_content_document(scope, handle, document);
    }
    Some(document)
}

pub(in crate::native_bridge) fn detached_iframe_content_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let cache_object = detached_iframe_cache_object(scope, iframe).unwrap_or(iframe);
    if let Some((runtime_ptr, handle)) = detached_iframe_runtime_and_handle(scope, iframe)
        && let Some(cached) =
            unsafe { &*runtime_ptr }.cached_detached_iframe_content_window(scope, handle)
    {
        return Some(cached);
    }
    if let Some(cached) =
        get_private_value(scope, cache_object, DETACHED_IFRAME_CONTENT_WINDOW_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(cached);
    }
    if !detached_iframe_has_cached_content_document(scope, iframe, cache_object)
        && let Some((runtime_ptr, handle)) =
            live_child_browsing_context_handle_for_detached_iframe(scope, iframe)
    {
        return unsafe { &mut *runtime_ptr }.child_browsing_context_window_wrapper(scope, handle);
    }
    let document = detached_iframe_content_document(scope, iframe)?;
    let base_url = get_private_value(scope, cache_object, DETACHED_IFRAME_BASE_URL_SLOT)
        .or_else(|| document.get(scope, v8str(scope, "baseURI").into()))
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| Url::parse(&value).ok())?;
    let window = build_detached_iframe_content_window(scope, iframe, document, &base_url)?;
    set_private_value(
        scope,
        cache_object,
        DETACHED_IFRAME_CONTENT_WINDOW_SLOT,
        window.into(),
    );
    if let Some((runtime_ptr, handle)) = detached_iframe_runtime_and_handle(scope, iframe) {
        unsafe { &mut *runtime_ptr }
            .set_cached_detached_iframe_content_window(scope, handle, window);
    }
    Some(window)
}

fn detached_iframe_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) -> Option<crate::native_bridge::ChildBrowsingContextSnapshot> {
    if let Some(srcdoc) =
        detached_element_attribute_value(scope, iframe, "srcdoc").filter(|value| !value.is_empty())
    {
        return Some(crate::native_bridge::ChildBrowsingContextSnapshot::html(
            detached_iframe_srcdoc_base_url(scope, iframe)?,
            srcdoc,
        ));
    }
    let target = detached_element_attribute_value(scope, iframe, "src")
        .filter(|src| !src.trim().is_empty())
        .map(|src| resolve_detached_url_attribute(scope, iframe, &src))
        .unwrap_or_else(|| "about:blank".to_owned());
    let url = Url::parse(&target).ok()?;
    let (host_ptr, handle) = detached_iframe_runtime_and_handle(scope, iframe)?;
    let host = unsafe { &*host_ptr };
    host.materialize_child_snapshot_for_url_blocking(handle, &url)
}

fn detached_iframe_srcdoc_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iframe: v8::Local<'s, v8::Object>,
) -> Option<Url> {
    detached_owner_document_object(scope, iframe)
        .and_then(|document| document.get(scope, v8str(scope, "baseURI").into()))
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| Url::parse(&value).ok())
        .or_else(|| Url::parse("about:blank").ok())
}

fn snapshot_base_url(document_url: Url, markup: &str) -> Url {
    let document =
        HtmlParser::with_scripting_enabled(false).parse(document_url.clone(), markup.to_owned());
    document
        .document()
        .map(|doc| doc.base_url().clone())
        .unwrap_or(document_url)
}
