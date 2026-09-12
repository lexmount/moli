mod bindings;
mod body_methods;
mod cors;
mod materialize;

use super::headers::{
    HeadersGuard, build_headers_object_with_state, filter_headers_for_guard, headers_entries,
    headers_entries_from_init,
};
use super::*;

/// Fetch's main-fetch body filter, given an already normalized request method.
pub(crate) fn response_has_null_body(method: &str, status: u16) -> bool {
    matches!(method, "HEAD" | "CONNECT") || matches!(status, 101 | 103 | 204 | 205 | 304)
}

pub(in crate::network_host) use self::bindings::{ParsedResponseInit, parse_response_init};
pub(crate) use self::bindings::{build_error_response_object, response_constructor_callback};
pub(super) use self::body_methods::install_response_body_methods;
pub(crate) use self::cors::{
    FetchResponseSecurityViolation, cors_preflight_request_headers_for_origin,
    cors_request_origin_after_redirects, fetch_response_needs_orb_body_validation,
    filter_cors_exposed_response_headers_for_origin, is_cors_policy_failure_message,
    validate_cors_preflight_response_for_origin, validate_cors_response,
    validate_cors_response_for_origin,
    validate_cross_origin_embedder_and_document_isolation_policy,
    validate_cross_origin_resource_policy, validate_fetch_response_headers_for_origin,
    validate_fetch_response_security_policy_for_origin,
    validate_fetch_response_security_policy_with_body,
    validate_fetch_response_security_policy_with_body_classified_for_origin,
    validate_fetch_response_security_policy_with_body_for_origin, validated_opaque_response_body,
};
pub(crate) use self::materialize::{
    FetchResponseRequest, MaterializedResponseBody, MaterializedResponseHead,
    build_fetch_response_object_for_request_mode,
    build_fetch_response_object_from_body_source_for_request_mode_with_filter,
    build_fetch_response_object_from_stream_for_request_mode_with_filter,
    build_fetch_response_object_from_subresource_body_for_request_mode_with_filter,
    build_filtered_cached_response_object,
    build_navigation_preload_response_object_from_stream_for_request_mode,
    materialize_cache_response_object_head, materialize_response_object_body,
    materialize_response_object_body_with_chunk_callback,
    materialize_response_object_internal_head, materialized_body_bytes_from_value,
    set_filtered_response_internal_head,
};
#[cfg(test)]
pub(crate) use self::materialize::{materialize_response_object, materialize_response_object_head};

#[cfg(test)]
pub(crate) use self::materialize::{
    build_fetch_response_object_from_stream_for_request_mode,
    build_fetch_response_object_from_subresource_body_for_request_mode,
};
