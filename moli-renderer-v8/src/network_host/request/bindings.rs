mod init;

use self::init::{
    apply_request_init_overrides, request_initial_state, validate_request_body_is_usable,
};
use super::input::request_headers_guard_for_mode;
use super::*;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Request)]
struct RequestInstanceDeclaration<'scope> {
    #[webapi(slot = REQUEST_METHOD_SLOT)]
    method: String,
    #[webapi(slot = REQUEST_URL_SLOT)]
    url: String,
    #[webapi(slot = REQUEST_HEADERS_SLOT)]
    headers: v8::Local<'scope, v8::Object>,
    #[webapi(slot = REQUEST_DESTINATION_SLOT, init = "")]
    destination: (),
    #[webapi(slot = REQUEST_REFERRER_SLOT)]
    referrer: String,
    #[webapi(slot = REQUEST_REFERRER_POLICY_SLOT)]
    referrer_policy: String,
    #[webapi(slot = REQUEST_MODE_SLOT)]
    mode: String,
    #[webapi(slot = REQUEST_CREDENTIALS_SLOT)]
    credentials: String,
    #[webapi(slot = REQUEST_CACHE_SLOT)]
    cache: String,
    #[webapi(slot = REQUEST_REDIRECT_SLOT)]
    redirect: String,
    #[webapi(slot = REQUEST_INTEGRITY_SLOT)]
    integrity: String,
    #[webapi(slot = REQUEST_KEEPALIVE_SLOT)]
    keepalive: bool,
    #[webapi(slot = REQUEST_PRIORITY_SLOT)]
    priority: String,
    #[webapi(slot = REQUEST_SIGNAL_SLOT)]
    signal: v8::Local<'scope, v8::Value>,
    #[webapi(slot = REQUEST_DUPLEX_SLOT)]
    duplex: String,
    #[webapi(slot = REQUEST_IS_HISTORY_NAVIGATION_SLOT, init = false)]
    is_history_navigation: (),
    #[webapi(slot = REQUEST_IS_RELOAD_NAVIGATION_SLOT, init = false)]
    is_reload_navigation: (),
    #[webapi(slot = REQUEST_BODY_SLOT)]
    body: v8::Local<'scope, v8::Value>,
    #[webapi(slot = REQUEST_BODY_USED_SLOT, init = false)]
    body_used: (),
}

pub(crate) fn request_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Request': Please use the 'new' operator.",
        );
        return;
    }

    let obj = args.this();
    let data = args.data();
    let child_handle = callback_child_handle(scope, data);
    let base_url = callback_base_url(scope, data);
    let mut state = match request_initial_state(scope, &args, child_handle, base_url) {
        Ok(state) => state,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };

    let init_arg = args.get(1);
    if !init_arg.is_null_or_undefined()
        && let Ok(init) = v8::Local::<v8::Object>::try_from(init_arg)
        && let Err(error) = apply_request_init_overrides(scope, init, &mut state)
    {
        webidl::throw_error(scope, &error);
        return;
    }
    if let Err(error) = validate_request_body_is_usable(scope, &state) {
        webidl::throw_error(scope, &error);
        return;
    }

    // Fresh URL entry lookup follows RequestInit conversion. Inherited URL
    // entries, including unavailable entries, survive getters that revoke it.
    let blob_url_entry = state.blob_url_entry.or_else(|| {
        url::Url::parse(&state.url_resolved)
            .ok()
            .and_then(|url| CapturedBlobUrl::capture(&url))
    });
    set_blob_url_entry(scope, obj, blob_url_entry);
    append_default_body_content_type(&mut state.headers, state.body_content_type.as_deref());
    let body_buffer = state
        .body
        .take()
        .and_then(|body| set_network_body_owned_bytes(scope, obj, body));

    let headers_obj = build_headers_object_with_state(
        scope,
        &state.headers,
        request_headers_guard_for_mode(&state.mode),
        false,
    );
    install_headers_object_methods(scope, headers_obj);

    let signal_source = state
        .signal
        .as_ref()
        .map(|signal| v8::Local::new(scope, signal));
    let signal = match new_abort_signal_for_request_with_source(scope, signal_source) {
        Some(signal) => signal,
        None if state.signal.is_some() => return,
        None => v8::undefined(scope).into(),
    };
    let body_value = if let Some(stream) = state.body_stream.as_ref() {
        let stream = v8::Local::new(scope, stream);
        let stream = if state.inherited_body_stream {
            let Some(proxy) = crate::context_bootstrap::proxy_fetch_body_stream(scope, stream)
            else {
                return;
            };
            proxy
        } else {
            stream
        };
        stream.into()
    } else {
        body_buffer
            .and_then(|buffer| {
                new_readable_stream_from_array_buffer(scope, buffer, buffer.byte_length())
            })
            .map(|stream| stream.into())
            .unwrap_or_else(|| v8::null(scope).into())
    };
    RequestInstanceDeclaration::new(
        state.method,
        state.url_resolved,
        headers_obj,
        state.referrer,
        state.referrer_policy,
        state.mode,
        state.credentials,
        state.cache,
        request_redirect_mode_label(state.redirect_mode).to_owned(),
        state.integrity,
        state.keepalive,
        state.priority.as_ref().to_owned(),
        signal,
        state.duplex,
        body_value,
    )
    .initialize(scope, obj)
    .expect("Request instance declaration should initialize");
    mark_request_object(scope, obj);

    rv.set(obj.into());
}

fn callback_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<url::Url> {
    if !value.is_string() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| url::Url::parse(&value).ok())
}

fn callback_child_handle(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<crate::document_runtime::DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless
            .then_some(index)
            .and_then(|index| usize::try_from(index).ok())
            .map(crate::document_runtime::DomHandle::new);
    }
    value
        .integer_value(scope)
        .filter(|index| *index >= 0)
        .and_then(|index| usize::try_from(index).ok())
        .map(crate::document_runtime::DomHandle::new)
}
