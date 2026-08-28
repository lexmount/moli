mod async_fetch;
mod beacon;
mod bindings;
mod body;
mod body_source;
mod browser_response;
mod csp_reports;
mod event_source;
mod fetch;
mod fetch_surface;
mod headers;
mod image;
mod js_values;
mod media;
mod preflight_events;
mod request;
mod request_scope;
mod response;
mod stylesheet_subresource;
mod text_track;
mod url_helpers;
mod xhr;

use http::StatusCode;
use moli_fetch::{Request, Response, observe_cookie_access_report_for_request};
use moli_webapi_declare::WebApiObject;

use crate::network::ResourceRequestClient;

pub(crate) use self::async_fetch::{
    browser_request_needs_manual_preflight_redirects,
    fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata,
    fetch_browser_subresource_with_preflight_and_network_metadata,
    fetch_browser_subresource_with_preflight_headers,
    fetch_browser_subresource_with_preflight_headers_and_network_metadata,
    spawn_async_subresource_fetch, spawn_async_subresource_fetch_with_redirect_chain,
};
pub(crate) use self::beacon::{navigator_send_beacon_callback, send_link_audit_ping};
pub(super) use self::bindings::install_window_network_bindings;
pub(in crate::network_host) use self::body::{PreparedBodyInit, body_init};
pub(crate) use self::body::{append_default_body_content_type, has_header};
#[cfg(test)]
pub(crate) use self::body_source::pending_network_body_source_buffered_len_for_test;
pub(in crate::network_host) use self::body_source::{
    BODY_FORM_DATA_UNSUPPORTED_CONTENT_TYPE_ERROR_TEXT, NetworkBodyConsumption,
    NetworkBodyConsumptionKind, clone_filtered_response_internal_body_source,
    clone_pending_network_body_stream, consume_filtered_response_internal_body_value_from_object,
    consume_filtered_response_internal_body_value_from_object_with_chunk_callback,
    consume_network_body_value_from_object,
    consume_network_body_value_from_object_with_chunk_callback, network_body_source_from_object,
    network_body_source_object_from_bytes, network_body_stream_from_response_body,
    network_body_stream_from_subresource_body, network_body_value_is_pending_stream,
    set_filtered_response_internal_body_from_bytes,
    set_filtered_response_internal_body_from_pending_stream,
    set_filtered_response_internal_body_from_response_body,
    set_filtered_response_internal_body_from_subresource_body, set_network_body_owned_bytes,
    take_network_body_bytes_from_object, try_network_body_bytes_from_object,
    try_network_body_value_from_object,
};
pub(crate) use self::body_source::{
    PendingNetworkBodySourceState, close_pending_network_body_stream,
    enqueue_pending_network_body_chunk, error_pending_network_body_stream,
    error_pending_network_body_stream_with_reason, new_network_body_source_id,
    pending_network_body_stream,
};
pub(in crate::network_host) use self::browser_response::http_status_text;
pub(crate) use self::browser_response::{local_url_response, local_url_response_result};
pub(crate) use self::csp_reports::{
    WindowCspReportRequestContext, capture_window_csp_report_request_context,
    send_content_security_policy_reports_for_window,
    send_content_security_policy_violation_report_from_window_context,
};
pub(crate) use self::event_source::{
    EVENT_SOURCE_CLOSED, EventSourceMessage, EventSourceParser, EventSourceTerminalMode,
    dispatch_event_source_message, event_source_constructor_callback, event_source_last_event_id,
    event_source_ready_state, event_source_reconnect_delay_ms, event_source_response_error,
    fail_event_source_connection, install_event_source_bindings, open_event_source_connection,
    update_event_source_stream_state,
};
pub(crate) use self::fetch_surface::set_response_slot_string;
pub(in crate::network_host) use self::fetch_surface::{
    REQUEST_BODY_SLOT, REQUEST_BODY_USED_SLOT, REQUEST_CACHE_SLOT, REQUEST_CREDENTIALS_SLOT,
    REQUEST_DESTINATION_SLOT, REQUEST_DUPLEX_SLOT, REQUEST_HEADERS_SLOT, REQUEST_INTEGRITY_SLOT,
    REQUEST_IS_HISTORY_NAVIGATION_SLOT, REQUEST_IS_RELOAD_NAVIGATION_SLOT, REQUEST_KEEPALIVE_SLOT,
    REQUEST_METHOD_SLOT, REQUEST_MODE_SLOT, REQUEST_PRIORITY_SLOT, REQUEST_REDIRECT_SLOT,
    REQUEST_REFERRER_POLICY_SLOT, REQUEST_REFERRER_SLOT, REQUEST_SIGNAL_SLOT,
    constructor_prototype, new_abort_signal_for_request_with_source,
};
pub(crate) use self::fetch_surface::{
    REQUEST_URL_SLOT, is_branded_request_object, request_headers_entries, request_method,
    request_slot_string, set_response_slot_bool,
};
pub(crate) use self::fetch_surface::{
    RESPONSE_BODY_SLOT, RESPONSE_BODY_USED_SLOT, RESPONSE_HEADERS_SLOT, RESPONSE_OK_SLOT,
    RESPONSE_REDIRECTED_SLOT, RESPONSE_STATUS_SLOT, RESPONSE_STATUS_TEXT_SLOT, RESPONSE_TYPE_SLOT,
    RESPONSE_URL_SLOT, is_branded_response_object,
};
pub(crate) use self::fetch_surface::{
    consume_webassembly_streaming_response_value, initialize_fetch_realm_helpers,
    install_request_bindings, install_response_bindings,
    set_request_destination_for_service_worker_fetch_event,
    set_request_mode_for_service_worker_fetch_event,
    set_request_reload_navigation_for_service_worker_fetch_event,
};
pub(in crate::network_host) use self::fetch_surface::{
    mark_request_object, request_slot_bool, request_slot_object, request_slot_value,
    response_slot_bool, response_slot_object, response_slot_string, response_slot_value,
    set_request_slot_bool, set_response_slot_value,
};
pub(crate) use self::headers::headers_constructor_callback;
pub(crate) use self::headers::install_headers_template_bindings;
pub(crate) use self::headers::{HeadersGuard, filter_headers_for_guard};
pub(crate) use self::image::{
    ImageElementResourceFetchStart, ScannedImagePreloadStart, image_response_descriptor,
    start_image_element_resource_fetch, start_scanned_image_preload,
};
pub(in crate::network_host) use self::js_values::{defined_object_string_property, v8_json_parse};
pub(crate) use self::media::{
    MediaElementResourceFetchStart, media_response_status_is_successful,
    start_media_element_resource_fetch,
};
pub(in crate::network_host) use self::preflight_events::CorsPreflightNetworkObserver;
pub(in crate::network_host) use self::request::normalize_request_method;
pub(crate) use self::request::request_constructor_callback;
pub(crate) use self::request::{
    mark_request_input_body_used_for_fetch, request_input_snapshot,
    try_resolve_request_constructor_url, try_resolve_request_constructor_url_for_base,
    try_resolve_request_constructor_url_for_child,
};
pub(crate) use self::request::{
    parse_fetch_init, parse_request_redirect_mode_label, request_object_credentials_mode,
};
pub(crate) use self::request_scope::effective_subresource_policy_context;
pub(in crate::network_host) use self::request_scope::{
    XHR_CHILD_CONTEXT_HANDLE_SLOT, active_subresource_network_partition_key,
    effective_subresource_referrer_policy, effective_subresource_request_scope,
    observe_subresource_request_cookie_report, subresource_request_scope_for_owner,
};
#[cfg(test)]
pub(crate) use self::response::materialize_response_object;
pub(crate) use self::response::{
    FetchResponseSecurityViolation, MaterializedResponseBody, MaterializedResponseHead,
    build_fetch_response_object_for_request_mode,
    build_fetch_response_object_from_body_source_for_request_mode,
    build_fetch_response_object_from_body_source_for_request_mode_with_filter,
    build_fetch_response_object_from_stream_for_request_mode,
    build_fetch_response_object_from_subresource_body_for_request_mode,
    build_filtered_cached_response_object,
    build_navigation_preload_response_object_from_stream_for_request_mode,
    cors_preflight_request_headers, filter_cors_exposed_response_headers,
    is_cors_policy_failure_message, materialize_response_object_body,
    materialize_response_object_body_with_chunk_callback, materialize_response_object_head,
    materialize_response_object_head_for_service_worker_respond_with,
    materialized_body_bytes_from_value, response_constructor_callback,
    validate_cors_preflight_response, validate_cors_response,
    validate_cross_origin_embedder_and_document_isolation_policy,
    validate_cross_origin_resource_policy, validate_fetch_response_security_policy,
    validate_fetch_response_security_policy_with_body,
    validate_fetch_response_security_policy_with_body_classified,
};
pub(crate) use self::stylesheet_subresource::{
    StylesheetSubresourceFetchStart, start_stylesheet_subresource_fetch,
};
pub(crate) use self::text_track::{
    TextTrackResourceFetchStart, start_text_track_resource_fetch, text_track_response_result,
};
pub(in crate::network_host) use self::url_helpers::merge_subresource_request_headers;
pub(crate) use self::url_helpers::resolve_context_url;
#[cfg(test)]
pub(crate) use self::xhr::prepare_xhr_send_body;
pub(crate) use self::xhr::{
    PreparedXhrSendBody, XHR_ABORTED_SLOT, XHR_ACTIVE_INTERNAL_ID_SLOT, XHR_ASYNC_SLOT,
    XHR_METHOD_SLOT, XHR_OPEN_GENERATION_SLOT, XHR_READY_STATE_SLOT, XHR_SEND_FLAG_SLOT,
    XHR_TIMEOUT_SLOT, XHR_TIMEOUT_START_MS_SLOT, XHR_TIMEOUT_TIMER_SLOT, XHR_URL_SLOT,
    XHR_WITH_CREDENTIALS_SLOT, apply_xhr_failure, apply_xhr_response,
    apply_xhr_response_body_source, apply_xhr_response_body_source_with_status_text,
    apply_xhr_streaming_response_body_source, apply_xhr_streaming_response_chunk,
    apply_xhr_streaming_response_head, apply_xhr_timeout, dispatch_xhr_upload_abort_if_in_progress,
    dispatch_xhr_upload_complete, finalize_xml_http_request_event_target_realm_bindings,
    install_progress_event_template_bindings, install_window_xml_http_request_template_bindings,
    install_xml_http_request_bindings, install_xml_http_request_event_target_bindings,
    prepare_xhr_send_body_from_args, progress_event_constructor_callback,
    reset_xhr_response_for_request_error, set_xhr_state_bool, set_xhr_state_number,
    throw_synchronous_xhr_failure, xhr_author_request_headers, xhr_constructor_callback,
    xhr_dispatch_progress_event, xhr_ensure_send_allowed, xhr_state_bool_property,
    xhr_state_number_property, xhr_state_string_property,
};
pub(in crate::network_host) const NETWORK_BODY_SLOT: &str = "__lmBody";
pub(in crate::network_host) const NETWORK_BODY_BYTES_SLOT: &str = "__lmBodyBytes";
pub(in crate::network_host) const NETWORK_BODY_SOURCE_SLOT: &str = "__lmNetworkBodySource";
pub(in crate::network_host) const NETWORK_BODY_SOURCE_KIND_SLOT: &str = "__lmNetworkBodySourceKind";
pub(in crate::network_host) const BODY_STREAM_CONSUMER_SLOT: &str = "__lmConsumeReadableStreamBody";
pub(crate) const BLOCKED_BY_CLIENT_ERROR_TEXT: &str = "net::ERR_BLOCKED_BY_CLIENT";
pub(crate) const FILE_NOT_FOUND_ERROR_TEXT: &str = "net::ERR_FILE_NOT_FOUND";
pub(crate) const FAILED_ERROR_TEXT: &str = "net::ERR_FAILED";
pub(crate) use crate::protocol_types::extract_subresource_auth_challenge;
pub(crate) use moli_fetch::NET_ERR_ABORTED_ERROR_TEXT as ABORTED_ERROR_TEXT;

#[derive(WebApiObject)]
#[webapi(interface = "Response")]
struct ChildResponseConstructorDeclaration<'scope> {
    data: v8::Local<'scope, v8::Value>,

    #[webapi(
        method = "redirect",
        length = 1,
        callback = self::fetch_surface::response_static_redirect_callback,
        data = self.data
    )]
    redirect: (),
}

pub(crate) fn install_fetch_constructors_for_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    base_url: &url::Url,
) {
    let Some(data) = v8_string(scope, base_url.as_str()).map(Into::into) else {
        return;
    };
    install_fetch_constructors_with_data(scope, window, data);
}

fn install_fetch_constructors_with_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    let parent_request = window
        .get(scope, v8str(scope, "Request").into())
        .or_else(|| global.get(scope, v8str(scope, "Request").into()))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    if let Some(request) = v8::Function::builder(request_constructor_callback)
        .length(1)
        .data(data)
        .build(scope)
    {
        let prototype = parent_request
            .and_then(|parent_request| parent_request.get(scope, v8str(scope, "prototype").into()))
            .or_else(|| constructor_prototype(scope, global, "Request").map(Into::into));
        if let Some(prototype) = prototype {
            let _ = request.set(scope, v8str(scope, "prototype").into(), prototype);
        }
        let _ = window.set(scope, v8str(scope, "Request").into(), request.into());
    }

    let parent_response = window
        .get(scope, v8str(scope, "Response").into())
        .or_else(|| global.get(scope, v8str(scope, "Response").into()))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok());
    if let Some(response) = v8::Function::builder(response_constructor_callback)
        .data(data)
        .build(scope)
    {
        let prototype = parent_response
            .and_then(|parent_response| {
                parent_response.get(scope, v8str(scope, "prototype").into())
            })
            .or_else(|| constructor_prototype(scope, global, "Response").map(Into::into));
        if let Some(prototype) = prototype {
            let _ = response.set(scope, v8str(scope, "prototype").into(), prototype);
        }
        if let Some(parent_response) = parent_response {
            for name in ["error", "json"] {
                if let Some(value) = parent_response.get(scope, v8str(scope, name).into()) {
                    let _ = response.set(scope, v8str(scope, name).into(), value);
                }
            }
        }
        ChildResponseConstructorDeclaration::new(data)
            .initialize(scope, response.into())
            .expect("child Response constructor declaration should initialize");
        let _ = window.set(scope, v8str(scope, "Response").into(), response.into());
    }
}

use super::{
    blob,
    context_bootstrap::{new_readable_stream_from_array_buffer, new_readable_stream_from_source},
    dom_parser,
    exception_reporting::invoke_callback,
    native_bridge::JsContextHost,
    page_task_queue::RendererResourceCompletionSender,
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceFetchEvent,
        AsyncSubresourceNetworkContext, AsyncSubresourceStreamingChunk,
        AsyncSubresourceStreamingFinished, AsyncSubresourceStreamingStarted,
        PendingSubresourceFetchInfo, SubresourceNetworkRecord, SubresourceResourceType,
    },
    util::{
        context_host_ptr_from_global_bridge, enqueue_host_microtask, throw_type_error, v8_string,
        v8str,
    },
};
