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
    #[webapi(slot = REQUEST_DESTINATION_SLOT)]
    destination: String,
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
    #[webapi(slot = REQUEST_IS_HISTORY_NAVIGATION_SLOT)]
    is_history_navigation: bool,
    #[webapi(slot = REQUEST_IS_RELOAD_NAVIGATION_SLOT)]
    is_reload_navigation: bool,
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
        String::new(),
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
        false,
        false,
        body_value,
    )
    .initialize(scope, obj)
    .expect("Request instance declaration should initialize");
    mark_request_object(scope, obj);

    rv.set(obj.into());
}

pub(crate) fn cached_request_from_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> moli_storage_service::StorageBucketCachedRequest {
    use moli_storage_service::{StorageBucketCachedRequest, StorageBucketCachedRequestMetadata};
    let defaults = StorageBucketCachedRequestMetadata::default();
    StorageBucketCachedRequest {
        method: request_method(scope, request),
        headers: request_headers_entries(scope, request),
        metadata: StorageBucketCachedRequestMetadata {
            destination: request_slot_string(scope, request, REQUEST_DESTINATION_SLOT)
                .unwrap_or(defaults.destination),
            referrer: request_slot_string(scope, request, REQUEST_REFERRER_SLOT)
                .unwrap_or(defaults.referrer),
            referrer_policy: request_slot_string(scope, request, REQUEST_REFERRER_POLICY_SLOT)
                .unwrap_or(defaults.referrer_policy),
            mode: request_slot_string(scope, request, REQUEST_MODE_SLOT).unwrap_or(defaults.mode),
            credentials: request_slot_string(scope, request, REQUEST_CREDENTIALS_SLOT)
                .unwrap_or(defaults.credentials),
            cache: request_slot_string(scope, request, REQUEST_CACHE_SLOT)
                .unwrap_or(defaults.cache),
            redirect: request_slot_string(scope, request, REQUEST_REDIRECT_SLOT)
                .unwrap_or(defaults.redirect),
            integrity: request_slot_string(scope, request, REQUEST_INTEGRITY_SLOT)
                .unwrap_or(defaults.integrity),
            keepalive: request_slot_bool(scope, request, REQUEST_KEEPALIVE_SLOT),
            priority: request_slot_string(scope, request, REQUEST_PRIORITY_SLOT)
                .unwrap_or(defaults.priority),
            duplex: request_slot_string(scope, request, REQUEST_DUPLEX_SLOT)
                .unwrap_or(defaults.duplex),
            is_history_navigation: request_slot_bool(
                scope,
                request,
                REQUEST_IS_HISTORY_NAVIGATION_SLOT,
            ),
            is_reload_navigation: request_slot_bool(
                scope,
                request,
                REQUEST_IS_RELOAD_NAVIGATION_SLOT,
            ),
        },
    }
}

/// Create Cache.keys() results directly from the stored internal request.
/// RequestInit conversion would reject navigation modes, clear destinations,
/// and resolve referrers against the querying realm instead of preserving them.
pub(crate) fn build_cached_request_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    url: &str,
    request: &moli_storage_service::StorageBucketCachedRequest,
) -> Option<v8::Local<'s, v8::Object>> {
    let metadata = &request.metadata;
    let headers = build_headers_object_with_state(
        scope,
        &request.headers,
        request_headers_guard_for_mode(&metadata.mode),
        true,
    );
    let signal = new_abort_signal_for_request_with_source(scope, None)?;
    RequestInstanceDeclaration::new(
        request.method.clone(),
        url.to_owned(),
        headers,
        metadata.destination.clone(),
        metadata.referrer.clone(),
        metadata.referrer_policy.clone(),
        metadata.mode.clone(),
        metadata.credentials.clone(),
        metadata.cache.clone(),
        metadata.redirect.clone(),
        metadata.integrity.clone(),
        metadata.keepalive,
        metadata.priority.clone(),
        signal,
        metadata.duplex.clone(),
        metadata.is_history_navigation,
        metadata.is_reload_navigation,
        v8::null(scope).into(),
    )
    .bind(scope)
    .ok()
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
