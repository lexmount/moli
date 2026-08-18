//! HTTP transport primitives for Moli.
//!
//! This crate provides fetch configuration, request/response models, async HTTP
//! execution, streaming HTML responses, and cookie-aware helper utilities used
//! by higher-level browser code.

mod blocking;
mod cancellation;
mod client;
mod client_hints;
mod config;
mod dns;
mod error;
mod headers;
mod network_fetch_result;
mod proxy_connect;
mod referrer_policy;
mod request;
mod request_policy;
mod response;
mod runtime;
mod streaming_response;
#[cfg(test)]
mod tests;
mod url_pattern;

#[cfg(any(test, feature = "test-support"))]
pub use blocking::{
    StreamingHtmlResponseStart, StreamingResponseCollector, outgoing_request_headers,
};
pub use blocking::{
    clear_http_cache, clear_http_cache_for_origin, clear_http_cache_root,
    clear_http_cache_root_for_origin, cookie_header_for_request, http_cache_stats,
    observe_cookie_access_report_for_request, trim_http_cache,
};
pub use cancellation::FetchCancelHandle;
pub use client::{FetchClient, FetchClientHandle};
pub use config::FetchConfig;
pub use error::{NET_ERR_ABORTED_ERROR_TEXT, ensure_http_status_success};
pub use headers::{
    cors_unsafe_request_header_names, is_cors_safelisted_method,
    is_cors_safelisted_request_content_type, is_cors_safelisted_request_header,
    is_cors_safelisted_request_range, is_cors_unsafe_request_header_byte,
    is_forbidden_request_header_name, is_forbidden_request_header_override_value,
    is_forbidden_response_header_name, is_no_cors_safelisted_request_header,
};
pub use moli_cookie_jar::SharedBrowserCookieStore as SharedCookieStore;
pub use moli_web_bot_auth::{WebBotAuthProfile, WebBotAuthSigner};
pub use network_fetch_result::{
    NetworkExchangeObservation, NetworkFetchFailureContext, NetworkFetchFailureRequestContext,
    NetworkFetchResult, NetworkObservationJournal, NetworkRequestObservation,
    NetworkResponseObservation,
};
pub use referrer_policy::{
    DEFAULT_REFERRER_POLICY, origin_referrer_url, referrer_header_value, sanitized_referrer_url,
};
pub use request::{
    BrowserNavigationRequestKind, BrowserRequestMetadata, FetchPriorityHint, Request, RequestAuth,
    RequestAuthScheme, RequestAuthTarget, RequestCacheMode, RequestCredentialsMode, RequestMode,
    RequestPriorityHints, RequestRedirectMode, RequestResourceType, ResourceLoadPriority,
    ScriptFetchRequestMetadata, ScriptFetchSchedulerPriority, SubresourceRequestMetadata,
};
pub use request_policy::{is_bad_port, should_request_be_blocked_due_to_bad_port};
pub use response::{
    NegotiatedHttpVersion, NetworkRequestExtraInfo, NetworkResponseExtraInfo, RawResponse,
    RedirectInfo, Response, ResponseBody, ResponseHead,
};
pub use runtime::PendingStreamingRawResponse;
pub use runtime::{
    FetchRuntimeIdentity, FetchRuntimeJoinReport, FetchRuntimeJoinStatus, FetchRuntimePanicReport,
};
pub use streaming_response::{StreamingHtmlResponse, StreamingRawResponse};
pub use url_pattern::url_pattern_matches;
