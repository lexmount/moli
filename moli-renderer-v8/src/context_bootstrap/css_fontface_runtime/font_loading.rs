use super::font_face::{
    ensure_font_face_loaded_promise, font_face_loaded_resolver, font_face_status,
    font_face_string_slot, set_font_face_status,
};
use super::*;
use crate::util::{get_private_value, set_private_value};

const URLS: &str = "__moliFontFaceUrls";
const NEXT_URL: &str = "__moliFontFaceNextUrl";
const DATA: &str = "__moliFontFaceData";

pub(super) fn store_font_face_binary_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    bytes: Vec<u8>,
) {
    let backing = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let data = v8::ArrayBuffer::with_backing_store(scope, &backing);
    set_private_value(scope, face, DATA, data.into());
}

pub(super) fn capture_font_face_sources<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    let source = font_face_string_slot(scope, face, FONT_FACE_SOURCE_SLOT).unwrap_or_default();
    let base = crate::network_host::font_face_base_url(scope)
        .unwrap_or_else(|| url::Url::parse("about:blank").unwrap());
    let urls = crate::css_resource_urls::font_face_source_urls(&source, &base);
    let values: Vec<v8::Local<v8::Value>> = urls
        .iter()
        .filter_map(|url| v8_string(scope, url.as_str()).map(Into::into))
        .collect();
    let urls = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, face, URLS, urls.into());
}

pub(super) fn queue_font_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.queue_font_loading_task(scope, callback);
    } else if !crate::worker::queue_worker_font_task(scope, callback) {
        scope.enqueue_microtask(callback);
    }
}

pub(super) fn start_font_face_load<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    if font_face_status(scope, face).as_deref() != Some("unloaded") {
        return;
    }
    let Some(context) = face.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    ensure_font_face_loaded_promise(scope, face);
    set_font_face_status(scope, face, "loading");
    super::events::notify_font_face_set_owners_loading(scope, face);
    try_next_font_face_source(scope, face);
}

fn try_next_font_face_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) {
    let urls = get_private_value(scope, face, URLS)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok());
    let mut index = get_private_value(scope, face, NEXT_URL)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    if let Some(urls) = urls {
        while index < urls.length() {
            let source = urls
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
                .map(|value| value.to_rust_string_lossy(scope));
            index += 1;
            let next = v8::Integer::new_from_unsigned(scope, index);
            set_private_value(scope, face, NEXT_URL, next.into());
            if let Some(url) = source.and_then(|source| url::Url::parse(&source).ok())
                && crate::network_host::start_font_face_resource_fetch(scope, face, url).is_ok()
            {
                return;
            }
        }
    }
    if let Some(callback) = v8::Function::builder(font_face_failure_task)
        .data(face.into())
        .build(scope)
    {
        queue_font_task(scope, callback);
    }
}

fn font_face_failure_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(face) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if font_face_status(scope, face).as_deref() != Some("loading") {
        return;
    }
    let error = new_dom_exception_value(
        scope,
        "No source in the FontFace src list could be loaded.",
        "NetworkError",
    );
    set_private_value(scope, face, FONT_FACE_ERROR_SLOT, error);
    set_font_face_status(scope, face, "error");
    if let Some(resolver) = font_face_loaded_resolver(scope, face) {
        let _ = resolver.reject(scope, error);
    }
    super::events::notify_font_face_set_owners_finished(scope, face, false);
}

const PENDING_DATA: &str = "__moliFontFacePendingData";

pub(crate) fn complete_font_face_resource<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    bytes: Option<Vec<u8>>,
) {
    if font_face_status(scope, face).as_deref() != Some("loading") {
        return;
    }
    let data = if let Some(bytes) = bytes {
        let backing = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
        v8::ArrayBuffer::with_backing_store(scope, &backing).into()
    } else {
        v8::undefined(scope).into()
    };
    set_private_value(scope, face, PENDING_DATA, data);
    if let Some(callback) = v8::Function::builder(font_face_completion_task)
        .data(face.into())
        .build(scope)
    {
        queue_font_task(scope, callback);
    }
}

fn font_face_completion_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(face) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    if font_face_status(scope, face).as_deref() != Some("loading") {
        return;
    }
    let data = get_private_value(scope, face, PENDING_DATA)
        .and_then(|value| v8::Local::<v8::ArrayBuffer>::try_from(value).ok());
    let undefined = v8::undefined(scope);
    set_private_value(scope, face, PENDING_DATA, undefined.into());
    let Some(data) = data.filter(|buffer| {
        let bytes: Vec<u8> = buffer
            .get_backing_store()
            .iter()
            .map(|byte| byte.get())
            .collect();
        moli_layout::validate_web_font_bytes(&bytes).is_ok()
    }) else {
        try_next_font_face_source(scope, face);
        return;
    };
    set_private_value(scope, face, DATA, data.into());
    set_font_face_status(scope, face, "loaded");
    if let Some(resolver) = font_face_loaded_resolver(scope, face) {
        let _ = resolver.resolve(scope, face.into());
    }
    super::events::notify_font_face_set_owners_finished(scope, face, true);
}
