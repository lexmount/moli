//! Bootstrap the `DedicatedWorkerGlobalScope` — the JS global environment
//! visible inside a Web Worker.
//!
//! Workers have no DOM access.  The global exposes:
//! - `self` (the global itself)
//! - `postMessage(data)` — send to parent
//! - `onmessage` / `onmessageerror` and functional service worker handlers
//! - `close()` — request worker shutdown
//! - `console` (log/warn/error/info/profile/profileEnd)
//! - `setTimeout` / `clearTimeout` / `setInterval` / `clearInterval`
//! - `requestAnimationFrame` / `cancelAnimationFrame`
//! - `globalThis`

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::{
    RendererSyntheticResponseBody,
    broadcast_channel_runtime::SharedBroadcastChannelRegistry,
    content_security_policy::ContentSecurityPolicyReportingEndpoints,
    message_port_runtime::SharedMessagePortRegistry,
    runtime::{ServiceWorkerRegistrationId, ServiceWorkerVersionId},
    service_worker_runtime::ServiceWorkerRequestDestination,
    worker::WorkerGlobalKind,
};
use anyhow::{Result, anyhow};
use http::HeaderName;
use moli_cookie_jar::StoredCookieSetReport;
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, Request, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, Response, ResponseBody, ResponseHead,
    should_request_be_blocked_due_to_bad_port,
};
use moli_storage_key::MoliStorageKey;
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiFunctionTemplate, WebApiObject};
use moli_webidl_callback::WebIdlCallbackInterface;
use moli_websocket::{
    Command as WebSocketCommand, CommandSender as WebSocketCommandSender,
    ConnectOptions as WebSocketConnectOptions, Event as WebSocketEvent, spawn_connection,
    spawn_failed_connection, websocket_cookie_url,
};
use tokio::sync::mpsc;
use url::Url;

use super::{
    decode_data_url_script_source,
    handle::{
        WorkerConsoleMessage, WorkerFetchHandlerType, WorkerPendingFetchContinue,
        WorkerPendingSubresourceFetch, WorkerPendingXhrContinue, WorkerToParentMessage,
        WorkerWebSocketFrameEvent, WorkerWebSocketLifecycleEvent,
    },
};
use crate::context_bootstrap::WebCryptoTaskResult;
use crate::context_bootstrap::{
    WebCryptoRejection, install_simple_event_target_methods,
    install_simple_event_target_ordered_handlers, simple_object_event_listeners_snapshot,
    simple_object_event_set_ordered_handler,
};
use crate::network::loads::{ResourceLoadDisposition, ResourceLoadKind, ResourceLoadLease};
use crate::network_host::{
    ABORTED_ERROR_TEXT, BLOCKED_BY_CLIENT_ERROR_TEXT, FAILED_ERROR_TEXT,
    FetchResponseSecurityViolation, HeadersGuard, PreparedXhrSendBody, XHR_ABORTED_SLOT,
    XHR_ACTIVE_INTERNAL_ID_SLOT, XHR_ASYNC_SLOT, XHR_METHOD_SLOT, XHR_OPEN_GENERATION_SLOT,
    XHR_READY_STATE_SLOT, XHR_SEND_FLAG_SLOT, XHR_TIMEOUT_SLOT, XHR_TIMEOUT_START_MS_SLOT,
    XHR_TIMEOUT_TIMER_SLOT, XHR_URL_SLOT, XHR_WITH_CREDENTIALS_SLOT,
    append_default_body_content_type, apply_xhr_failure, apply_xhr_response,
    apply_xhr_response_body_source, apply_xhr_timeout,
    browser_request_needs_manual_preflight_redirects,
    build_fetch_response_object_from_body_source_for_request_mode,
    build_fetch_response_object_from_stream_for_request_mode,
    build_fetch_response_object_from_subresource_body_for_request_mode,
    close_pending_network_body_stream, dispatch_xhr_upload_abort_if_in_progress,
    dispatch_xhr_upload_complete, enqueue_pending_network_body_chunk,
    error_pending_network_body_stream_with_reason, extract_subresource_auth_challenge,
    fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata,
    fetch_browser_subresource_with_preflight_headers_and_network_metadata,
    filter_cors_exposed_response_headers, filter_headers_for_guard, has_header,
    is_cors_policy_failure_message, local_url_response, parse_fetch_init,
    prepare_xhr_send_body_from_args, request_input_snapshot, request_object_credentials_mode,
    reset_xhr_response_for_request_error, resolve_context_url, set_xhr_state_bool,
    set_xhr_state_number, throw_synchronous_xhr_failure, validate_cors_response,
    validate_fetch_response_security_policy,
    validate_fetch_response_security_policy_with_body_classified, xhr_author_request_headers,
    xhr_dispatch_progress_event, xhr_ensure_send_allowed, xhr_state_bool_property,
    xhr_state_number_property, xhr_state_string_property,
};
use crate::opfs_task_result::OpfsTaskResult;
use crate::protocol_types::{
    PendingSubresourceAuthInfo, PendingSubresourceContinueEvent, PendingSubresourceFetchInfo,
    PendingSubresourceResponseInfo, SubresourceNetworkRecord, SubresourceNetworkRequestHandle,
    SubresourceResourceType, SubresourceResponseBody, SubresourceResponseBodyWriter,
    WebSocketFrameDirection, WebSocketFrameOpcode,
};
use crate::queue_microtask::worker_queue_microtask_callback;
use crate::runtime::{
    ServiceWorkerClientFocusError, ServiceWorkerClientNavigateError,
    ServiceWorkerClientNavigateResult, ServiceWorkerClientQueryResult,
    ServiceWorkerClientQueryType, ServiceWorkerClientSnapshot, ServiceWorkerClientsOpenWindowError,
    ServiceWorkerClientsOpenWindowResult, ServiceWorkerEventId, ServiceWorkerFetchCompletion,
    ServiceWorkerFetchResult, ServiceWorkerGetNotificationsResult,
    ServiceWorkerLifecycleCompletion, ServiceWorkerMessageCompletion,
    ServiceWorkerNavigationPreloadState, ServiceWorkerNavigationPreloadStateError,
    ServiceWorkerPushGetSubscriptionResult, ServiceWorkerPushSubscribeResult,
    ServiceWorkerPushSubscriptionSnapshot, ServiceWorkerPushUnsubscribeResult,
    ServiceWorkerShowNotificationResult, ServiceWorkerSyncGetTagsResult,
    ServiceWorkerSyncRegistrationResult,
};
use crate::text_codec::TextCodecStore;
use crate::types::{BroadcastChannelId, DedicatedWorkerId, MessagePortId, NetworkBodySourceId};
use crate::util::{
    get_private_value, global_constructor_object, global_constructor_prototype, set_private_value,
    throw_type_error, v8_string, v8str,
};
use crate::webidl;
use crate::worker::abort::{
    WorkerAbortStore, worker_abort_error_value, worker_abort_signal_aborted,
    worker_abort_signal_id, worker_abort_signal_reason, worker_dom_exception_value,
};

mod content_security_policy;
mod fetch;
mod import_scripts;
mod timers;
mod xhr;

use content_security_policy::*;
pub(in crate::worker) use content_security_policy::{
    continue_pending_worker_csp_report, fail_pending_worker_csp_report,
    fulfill_pending_worker_csp_report,
};
pub(crate) use fetch::*;
use import_scripts::*;
use timers::*;
pub(crate) use xhr::*;

pub(super) fn dispatch_worker_csp_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    loader: &crate::network::context::WorkerResourceLoader,
    violation: &crate::content_security_policy::ContentSecurityPolicyUrlViolation,
) {
    content_security_policy::dispatch_worker_content_security_policy_violation_event(
        scope, loader, violation,
    );
}

pub(super) const WORKER_GLOBAL_LISTENERS_SLOT: &str = "__moliWorkerGlobalListeners";
pub(crate) const WORKER_STATE_SLOT: &str = "__workerState";
const WORKER_GLOBAL_ONMESSAGE_SLOT: &str = "__moliWorkerGlobalOnMessage";
const WORKER_GLOBAL_ONMESSAGEERROR_SLOT: &str = "__moliWorkerGlobalOnMessageError";
const WORKER_GLOBAL_ONERROR_SLOT: &str = "__moliWorkerGlobalOnError";
const WORKER_GLOBAL_ONCONNECT_SLOT: &str = "__moliWorkerGlobalOnConnect";
const WORKER_GLOBAL_ONINSTALL_SLOT: &str = "__moliWorkerGlobalOnInstall";
const WORKER_GLOBAL_ONACTIVATE_SLOT: &str = "__moliWorkerGlobalOnActivate";
const WORKER_GLOBAL_ONFETCH_SLOT: &str = "__moliWorkerGlobalOnFetch";
const WORKER_GLOBAL_ONPUSH_SLOT: &str = "__moliWorkerGlobalOnPush";
const WORKER_GLOBAL_ONSYNC_SLOT: &str = "__moliWorkerGlobalOnSync";
const WORKER_GLOBAL_ONPERIODICSYNC_SLOT: &str = "__moliWorkerGlobalOnPeriodicSync";
const WORKER_GLOBAL_ONNOTIFICATIONCLICK_SLOT: &str = "__moliWorkerGlobalOnNotificationClick";
const WORKER_GLOBAL_ONNOTIFICATIONCLOSE_SLOT: &str = "__moliWorkerGlobalOnNotificationClose";
const WORKER_GLOBAL_ONOFFLINE_SLOT: &str = "__moliWorkerGlobalOnOffline";
const WORKER_GLOBAL_ONONLINE_SLOT: &str = "__moliWorkerGlobalOnOnline";
const WORKER_GLOBAL_ONUNHANDLEDREJECTION_SLOT: &str = "__moliWorkerGlobalOnUnhandledRejection";
const WORKER_GLOBAL_ONREJECTIONHANDLED_SLOT: &str = "__moliWorkerGlobalOnRejectionHandled";
pub(super) const WORKER_EXCEPTION_SOURCE_SLOT: &str = "__moliWorkerExceptionSource";
pub(super) const WORKER_EXCEPTION_LINE_SLOT: &str = "__moliWorkerExceptionLine";
pub(super) const WORKER_EXCEPTION_COLUMN_SLOT: &str = "__moliWorkerExceptionColumn";
const SERVICE_WORKER_REGISTRATION_SCOPE_SLOT: &str = "__moliServiceWorkerRegistrationScope";
const SERVICE_WORKER_REGISTRATION_ID_SLOT: &str = "__moliServiceWorkerRegistrationId";
const SERVICE_WORKER_VERSION_ID_SLOT: &str = "__moliServiceWorkerVersionId";
const SERVICE_WORKER_WORKER_EVENTS_SLOT: &str = "__moliServiceWorkerWorkerEvents";
const SERVICE_WORKER_NAVIGATION_PRELOAD_MANAGER_SCOPE_SLOT: &str =
    "__moliServiceWorkerNavigationPreloadManagerScope";
const SERVICE_WORKER_CLIENT_ID_SLOT: &str = "__lmServiceWorkerClientId";
const WORKER_ORIGINAL_CONSOLE_SLOT: &str = "__moliWorkerOriginalConsole";

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct WorkerGlobalBootstrapPropertiesDeclaration<'scope> {
    #[webapi(slot = WORKER_STATE_SLOT)]
    worker_state: v8::Local<'scope, v8::External>,
    #[webapi(data_property = "self", readonly)]
    self_value: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = "crossOriginIsolated")]
    cross_origin_isolated: bool,
    #[webapi(data_property = "isSecureContext", readonly)]
    is_secure_context: bool,
    #[webapi(data_property = "globalThis")]
    global_this: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalNameDeclaration {
    #[webapi(data_property, enumerable)]
    name: String,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalOriginDeclaration {
    #[webapi(data_property, readonly)]
    origin: String,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalConsoleDeclaration<'scope> {
    #[webapi(data_property)]
    console: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalPerformanceDeclaration<'scope> {
    #[webapi(data_property)]
    performance: v8::Local<'scope, v8::Object>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerConsoleObjectDeclaration {
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "log"))]
    log: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "info"))]
    info: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "warn"))]
    warn: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "error"))]
    error: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "debug"))]
    debug: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "trace"))]
    trace: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    time: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    time_log: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    time_end: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    count: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    count_reset: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    dir: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    dirxml: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    table: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    group: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    group_end: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    group_collapsed: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    clear: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    assert: (),
    #[webapi(method, enumerable, callback = console_profile_callback)]
    profile: (),
    #[webapi(method, enumerable, callback = console_profile_end_callback)]
    profile_end: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerPerformanceObjectDeclaration {
    #[webapi(data_property, readonly)]
    time_origin: f64,
    #[webapi(method, callback = worker_performance_now_callback, data = self.time_origin)]
    now: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", enumerable)]
struct WorkerGlobalCommonEventHandlersDeclaration {
    #[webapi(
        accessor_property = "onerror",
        getter = worker_global_onerror_getter,
        setter = worker_global_onerror_setter
    )]
    onerror: (),
    #[webapi(
        accessor_property = "onoffline",
        getter = worker_global_onoffline_getter,
        setter = worker_global_onoffline_setter
    )]
    onoffline: (),
    #[webapi(
        accessor_property = "ononline",
        getter = worker_global_ononline_getter,
        setter = worker_global_ononline_setter
    )]
    ononline: (),
    #[webapi(
        accessor_property = "onunhandledrejection",
        getter = worker_global_onunhandledrejection_getter,
        setter = worker_global_onunhandledrejection_setter
    )]
    onunhandledrejection: (),
    #[webapi(
        accessor_property = "onrejectionhandled",
        getter = worker_global_onrejectionhandled_getter,
        setter = worker_global_onrejectionhandled_setter
    )]
    onrejectionhandled: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", enumerable)]
struct DedicatedWorkerGlobalEventHandlersDeclaration {
    #[webapi(
        accessor_property = "onmessage",
        getter = worker_global_onmessage_getter,
        setter = worker_global_onmessage_setter
    )]
    onmessage: (),
    #[webapi(
        accessor_property = "onmessageerror",
        getter = worker_global_onmessageerror_getter,
        setter = worker_global_onmessageerror_setter
    )]
    onmessageerror: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", enumerable)]
struct SharedWorkerGlobalEventHandlersDeclaration {
    #[webapi(
        accessor_property = "onconnect",
        getter = worker_global_onconnect_getter,
        setter = worker_global_onconnect_setter
    )]
    onconnect: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalCommonEventHandlerStateDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ONERROR_SLOT, init = "null")]
    onerror: (),
    #[webapi(slot = WORKER_GLOBAL_ONOFFLINE_SLOT, init = "null")]
    onoffline: (),
    #[webapi(slot = WORKER_GLOBAL_ONONLINE_SLOT, init = "null")]
    ononline: (),
    #[webapi(slot = WORKER_GLOBAL_ONUNHANDLEDREJECTION_SLOT, init = "null")]
    onunhandledrejection: (),
    #[webapi(slot = WORKER_GLOBAL_ONREJECTIONHANDLED_SLOT, init = "null")]
    onrejectionhandled: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct DedicatedWorkerGlobalEventHandlerStateDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ONMESSAGE_SLOT, init = "null")]
    onmessage: (),
    #[webapi(slot = WORKER_GLOBAL_ONMESSAGEERROR_SLOT, init = "null")]
    onmessageerror: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct SharedWorkerGlobalEventHandlerStateDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ONCONNECT_SLOT, init = "null")]
    onconnect: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "DedicatedWorkerGlobalScope", enumerable)]
struct DedicatedWorkerGlobalMethodsDeclaration {
    #[webapi(method, callback = worker_close_callback, length = 0)]
    close: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct DedicatedWorkerGlobalPostMessageDeclaration {
    #[webapi(method = "postMessage", callback = worker_post_message_callback, length = 1)]
    post_message: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "SharedWorkerGlobalScope", enumerable)]
struct SharedWorkerGlobalMethodsDeclaration {
    #[webapi(method, callback = worker_close_callback, length = 0)]
    close: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalCommonOperationsDeclaration {
    #[webapi(
        method = "structuredClone",
        callback = worker_structured_clone_callback,
        length = 1
    )]
    structured_clone: (),
    #[webapi(method, callback = worker_fetch_callback, length = 1)]
    fetch: (),
    #[webapi(method = "importScripts", callback = worker_import_scripts_callback, length = 0)]
    import_scripts: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalTimerOperationsDeclaration {
    #[webapi(method = "setTimeout", callback = worker_set_timeout_callback, length = 1)]
    set_timeout: (),
    #[webapi(method = "clearTimeout", callback = worker_clear_timeout_callback, length = 1)]
    clear_timeout: (),
    #[webapi(method = "setInterval", callback = worker_set_interval_callback, length = 1)]
    set_interval: (),
    #[webapi(method = "clearInterval", callback = worker_clear_interval_callback, length = 1)]
    clear_interval: (),
    #[webapi(
        method = "requestAnimationFrame",
        callback = worker_request_animation_frame_callback,
        length = 1
    )]
    request_animation_frame: (),
    #[webapi(
        method = "cancelAnimationFrame",
        callback = worker_clear_timeout_callback,
        length = 1
    )]
    cancel_animation_frame: (),
    #[webapi(
        method = "queueMicrotask",
        callback = worker_queue_microtask_callback,
        length = 1
    )]
    queue_microtask: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalCreateImageBitmapDeclaration {
    #[webapi(
        method = "createImageBitmap",
        callback = worker_create_image_bitmap_callback,
        length = 1
    )]
    create_image_bitmap: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", enumerable)]
struct ServiceWorkerGlobalEventHandlersDeclaration {
    #[webapi(
        accessor_property = "oninstall",
        getter = worker_global_oninstall_getter,
        setter = worker_global_oninstall_setter
    )]
    oninstall: (),
    #[webapi(
        accessor_property = "onactivate",
        getter = worker_global_onactivate_getter,
        setter = worker_global_onactivate_setter
    )]
    onactivate: (),
    #[webapi(
        accessor_property = "onfetch",
        getter = worker_global_onfetch_getter,
        setter = worker_global_onfetch_setter
    )]
    onfetch: (),
    #[webapi(
        accessor_property = "onpush",
        getter = worker_global_onpush_getter,
        setter = worker_global_onpush_setter
    )]
    onpush: (),
    #[webapi(
        accessor_property = "onsync",
        getter = worker_global_onsync_getter,
        setter = worker_global_onsync_setter
    )]
    onsync: (),
    #[webapi(
        accessor_property = "onperiodicsync",
        getter = worker_global_onperiodicsync_getter,
        setter = worker_global_onperiodicsync_setter
    )]
    onperiodicsync: (),
    #[webapi(
        accessor_property = "onmessage",
        getter = worker_global_onmessage_getter,
        setter = worker_global_onmessage_setter
    )]
    onmessage: (),
    #[webapi(
        accessor_property = "onmessageerror",
        getter = worker_global_onmessageerror_getter,
        setter = worker_global_onmessageerror_setter
    )]
    onmessageerror: (),
    #[webapi(
        accessor_property = "onnotificationclick",
        getter = worker_global_onnotificationclick_getter,
        setter = worker_global_onnotificationclick_setter
    )]
    onnotificationclick: (),
    #[webapi(
        accessor_property = "onnotificationclose",
        getter = worker_global_onnotificationclose_getter,
        setter = worker_global_onnotificationclose_setter
    )]
    onnotificationclose: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerGlobalEventHandlerStateDeclaration {
    #[webapi(slot = WORKER_GLOBAL_ONINSTALL_SLOT, init = "null")]
    oninstall: (),
    #[webapi(slot = WORKER_GLOBAL_ONACTIVATE_SLOT, init = "null")]
    onactivate: (),
    #[webapi(slot = WORKER_GLOBAL_ONFETCH_SLOT, init = "null")]
    onfetch: (),
    #[webapi(slot = WORKER_GLOBAL_ONPUSH_SLOT, init = "null")]
    onpush: (),
    #[webapi(slot = WORKER_GLOBAL_ONSYNC_SLOT, init = "null")]
    onsync: (),
    #[webapi(slot = WORKER_GLOBAL_ONPERIODICSYNC_SLOT, init = "null")]
    onperiodicsync: (),
    #[webapi(slot = WORKER_GLOBAL_ONMESSAGE_SLOT, init = "null")]
    onmessage: (),
    #[webapi(slot = WORKER_GLOBAL_ONMESSAGEERROR_SLOT, init = "null")]
    onmessageerror: (),
    #[webapi(slot = WORKER_GLOBAL_ONNOTIFICATIONCLICK_SLOT, init = "null")]
    onnotificationclick: (),
    #[webapi(slot = WORKER_GLOBAL_ONNOTIFICATIONCLOSE_SLOT, init = "null")]
    onnotificationclose: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerClientsDeclaration {
    #[webapi(method, callback = service_worker_clients_claim_callback, length = 0)]
    claim: (),
    #[webapi(method, callback = service_worker_clients_get_callback, length = 1)]
    get: (),
    #[webapi(
        method = "matchAll",
        callback = service_worker_clients_match_all_callback,
        length = 0
    )]
    match_all: (),
    #[webapi(
        method = "openWindow",
        callback = service_worker_clients_open_window_callback,
        length = 1
    )]
    open_window: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerGlobalRuntimeDeclaration<'scope> {
    #[webapi(data_property, readonly)]
    registration: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, readonly)]
    clients: v8::Local<'scope, v8::Object>,
    #[webapi(method = "skipWaiting", callback = service_worker_skip_waiting_callback, length = 0)]
    skip_waiting: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "ExtendableEvent",
    constructor_callback = extendable_event_constructor_callback,
    constructor_length = 1
)]
struct ExtendableEventTemplateDeclaration {
    #[webapi(method = "waitUntil", callback = extendable_event_wait_until_callback, length = 1)]
    wait_until: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "ExtendableMessageEvent",
    constructor_callback = extendable_message_event_constructor_callback,
    constructor_length = 1
)]
struct ExtendableMessageEventTemplateDeclaration {}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerGlobalScopeConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "WorkerGlobalScope")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct DedicatedWorkerGlobalScopeConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "DedicatedWorkerGlobalScope")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct SharedWorkerGlobalScopeConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "SharedWorkerGlobalScope")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerGlobalScopeConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "ServiceWorkerGlobalScope")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ExtendableEventConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "ExtendableEvent")]
    extendable_event: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ExtendableMessageEventConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "ExtendableMessageEvent")]
    extendable_message_event: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerScopePrototypeConstructorDeclaration<'scope> {
    #[webapi(data_property = "constructor")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerPrototypeTagDeclaration {
    #[webapi(to_string_tag, readonly)]
    tag: &'static str,
}

#[derive(WebApiObject)]
#[webapi(interface = "ServiceWorkerRegistration")]
struct ServiceWorkerGlobalRegistrationDeclaration<'scope> {
    #[webapi(data_property, readonly)]
    scope: String,

    #[webapi(
        accessor_property = "installing",
        getter = service_worker_registration_installing_getter
    )]
    installing: (),

    #[webapi(
        accessor_property = "waiting",
        getter = service_worker_registration_waiting_getter
    )]
    waiting: (),

    #[webapi(
        accessor_property = "active",
        getter = service_worker_registration_active_getter
    )]
    active: (),

    #[webapi(method, callback = service_worker_registration_unregister_callback, length = 0)]
    unregister: (),

    #[webapi(
        method = "showNotification",
        callback = service_worker_registration_show_notification_callback,
        length = 1
    )]
    show_notification: (),

    #[webapi(
        method = "getNotifications",
        callback = service_worker_registration_get_notifications_callback,
        length = 0
    )]
    get_notifications: (),

    #[webapi(data_property, readonly)]
    sync: v8::Local<'scope, v8::Object>,

    #[webapi(data_property = "periodicSync", readonly)]
    periodic_sync: v8::Local<'scope, v8::Object>,

    #[webapi(data_property = "pushManager", readonly)]
    push_manager: v8::Local<'scope, v8::Object>,

    #[webapi(data_property = "navigationPreload", readonly)]
    navigation_preload: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "ServiceWorker")]
struct ServiceWorkerGlobalServiceWorkerDeclaration {
    #[webapi(data_property = "scriptURL", readonly)]
    script_url: String,

    #[webapi(data_property, readonly)]
    state: String,

    #[webapi(method = "postMessage", callback = service_worker_worker_post_message_callback, length = 1)]
    post_message: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "SyncManager")]
struct ServiceWorkerGlobalSyncManagerDeclaration {
    #[webapi(
        method = "register",
        callback = service_worker_sync_manager_register_callback,
        length = 1
    )]
    register: (),

    #[webapi(
        method = "getTags",
        callback = service_worker_sync_manager_get_tags_callback,
        length = 0
    )]
    get_tags: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "PeriodicSyncManager")]
struct ServiceWorkerGlobalPeriodicSyncManagerDeclaration {
    #[webapi(
        method = "register",
        callback = service_worker_periodic_sync_manager_register_callback,
        length = 1
    )]
    register: (),

    #[webapi(
        method = "getTags",
        callback = service_worker_periodic_sync_manager_get_tags_callback,
        length = 0
    )]
    get_tags: (),

    #[webapi(
        method = "unregister",
        callback = service_worker_periodic_sync_manager_unregister_callback,
        length = 1
    )]
    unregister: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "PushManager")]
struct ServiceWorkerGlobalPushManagerDeclaration {
    #[webapi(
        method = "subscribe",
        callback = service_worker_push_manager_subscribe_callback,
        length = 0
    )]
    subscribe: (),

    #[webapi(
        method = "getSubscription",
        callback = service_worker_push_manager_get_subscription_callback,
        length = 0
    )]
    get_subscription: (),

    #[webapi(
        method = "permissionState",
        callback = service_worker_push_manager_permission_state_callback,
        length = 0
    )]
    permission_state: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "NavigationPreloadManager")]
struct ServiceWorkerGlobalNavigationPreloadManagerDeclaration {
    #[webapi(
        method,
        callback = service_worker_navigation_preload_manager_enable_callback,
        length = 0
    )]
    enable: (),

    #[webapi(
        method,
        callback = service_worker_navigation_preload_manager_disable_callback,
        length = 0
    )]
    disable: (),

    #[webapi(
        method = "setHeaderValue",
        callback = service_worker_navigation_preload_manager_set_header_value_callback,
        length = 1
    )]
    set_header_value: (),

    #[webapi(
        method = "getState",
        callback = service_worker_navigation_preload_manager_get_state_callback,
        length = 0
    )]
    get_state: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "PushSubscription")]
struct ServiceWorkerPushSubscriptionDeclaration<'scope> {
    #[webapi(data_property, readonly)]
    endpoint: String,

    #[webapi(data_property = "expirationTime", readonly)]
    expiration_time: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, readonly)]
    options: v8::Local<'scope, v8::Object>,

    #[webapi(method, callback = service_worker_push_subscription_unsubscribe_callback, length = 0)]
    unsubscribe: (),

    #[webapi(
        method = "toJSON",
        callback = service_worker_push_subscription_to_json_callback,
        length = 0
    )]
    to_json: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerPushSubscriptionOptionsDeclaration<'scope> {
    #[webapi(data_property = "userVisibleOnly", readonly)]
    user_visible_only: bool,

    #[webapi(data_property = "applicationServerKey", readonly)]
    application_server_key: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerNavigationPreloadStateDeclaration {
    #[webapi(data_property, enumerable)]
    enabled: bool,

    #[webapi(data_property = "headerValue", enumerable)]
    header_value: String,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct InitializedExtendableEventStateDeclaration<'scope> {
    #[webapi(data_property = "type", enumerable)]
    event_type: String,

    #[webapi(data_property, enumerable)]
    bubbles: bool,

    #[webapi(data_property, enumerable)]
    cancelable: bool,

    #[webapi(data_property, enumerable)]
    composed: bool,

    #[webapi(data_property = "defaultPrevented", enumerable)]
    default_prevented: bool,

    #[webapi(data_property, enumerable)]
    target: v8::Local<'scope, v8::Value>,

    #[webapi(data_property = "currentTarget", enumerable)]
    current_target: v8::Local<'scope, v8::Value>,

    #[webapi(data_property = "eventPhase", enumerable)]
    event_phase: i32,

    #[webapi(data_property = "isTrusted", enumerable)]
    is_trusted: bool,

    #[webapi(data_property = "timeStamp", enumerable)]
    time_stamp: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ExtendableMessageEventStateDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    origin: String,

    #[webapi(data_property = "lastEventId", enumerable)]
    last_event_id: String,

    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,

    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Client")]
pub(super) struct ServiceWorkerBaseClientDeclaration<'scope> {
    #[webapi(data_property, readonly)]
    id: String,

    #[webapi(data_property, readonly)]
    url: v8::Local<'scope, v8::String>,

    #[webapi(data_property = "type", readonly)]
    client_type: &'static str,

    #[webapi(method = "postMessage", callback = service_worker_client_post_message_callback, length = 1)]
    post_message: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "WindowClient")]
pub(super) struct ServiceWorkerWindowClientDeclaration<'scope> {
    #[webapi(data_property, readonly)]
    id: String,

    #[webapi(data_property, readonly)]
    url: v8::Local<'scope, v8::String>,

    #[webapi(data_property = "type", readonly)]
    client_type: &'static str,

    #[webapi(data_property = "frameType", readonly)]
    frame_type: &'static str,

    #[webapi(data_property = "lifecycleState", readonly)]
    lifecycle_state: &'static str,

    #[webapi(data_property = "visibilityState", readonly)]
    visibility_state: &'static str,

    #[webapi(data_property, readonly)]
    focused: bool,

    #[webapi(method = "postMessage", callback = service_worker_client_post_message_callback, length = 1)]
    post_message: (),

    #[webapi(method, callback = service_worker_window_client_focus_callback, length = 0)]
    focus: (),

    #[webapi(method, callback = service_worker_window_client_navigate_callback, length = 1)]
    navigate: (),
}

pub(super) struct PendingServiceWorkerClientQuery {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
    pub(super) query_type: PendingServiceWorkerClientQueryType,
}

pub(super) struct PendingServiceWorkerClientNavigate {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerClientFocus {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerClientsOpenWindow {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerShowNotification {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerGetNotifications {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerSyncRegistration {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerSyncGetTags {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerPeriodicSyncRegistration {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerPeriodicSyncGetTags {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerPeriodicSyncUnregistration {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerPushSubscribe {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "BackgroundSyncOptions")]
struct BackgroundSyncOptions {
    #[webidl(
        name = "minInterval",
        converter = "enforce_range_unsigned_long_long",
        default = 0
    )]
    min_interval: u64,
}

pub(super) struct PendingServiceWorkerPushGetSubscription {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

pub(super) struct PendingServiceWorkerPushUnsubscribe {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PendingServiceWorkerClientQueryType {
    Get,
    MatchAll,
}

/// A WebCrypto primitive dispatched off the worker event loop to the blocking
/// pool. The worker owns the resolver and matches the completion back by the
/// task id allocated for this worker lifetime.
pub(super) struct PendingWorkerWebCryptoTask {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
}

/// Completion of a worker WebCrypto blocking task, routed back onto the worker
/// event loop. The worker sink captures these ids at registration just as the
/// page-side typed producer captures its exact Page/Window owner.
pub(crate) struct WorkerWebCryptoCompletion {
    pub(crate) task_id: u64,
    pub(crate) result: Result<WebCryptoTaskResult, WebCryptoRejection>,
}

pub(super) struct PendingWorkerOpfsTask {
    pub(super) locator: moli_storage_service::StorageBucketLocator,
    pub(super) handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
    pub(super) settlement: crate::opfs_owner_tasks::OpfsTaskSettlement,
}

pub(super) struct WorkerOpfsOwnerState {
    next_task_id: u64,
    pending_tasks: HashMap<u64, PendingWorkerOpfsTask>,
    handles: crate::opfs_owner_tasks::OpfsHandleRegistry,
    directory_iterators: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry,
}

impl Default for WorkerOpfsOwnerState {
    fn default() -> Self {
        Self {
            next_task_id: 1,
            pending_tasks: HashMap::new(),
            handles: crate::opfs_owner_tasks::OpfsHandleRegistry::default(),
            directory_iterators: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry::default(),
        }
    }
}

impl WorkerOpfsOwnerState {
    pub(super) fn has_pending_tasks(&self) -> bool {
        !self.pending_tasks.is_empty()
    }
}

pub(crate) struct WorkerOpfsCompletion {
    pub(crate) task_id: u64,
    pub(crate) result: OpfsTaskResult,
}

pub(super) struct PendingWorkerFetch {
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
    pub(super) document_url: Url,
    pub(super) credentials_mode: RequestCredentialsMode,
    pub(super) request_mode: moli_fetch::RequestMode,
    pub(super) redirect_mode: RequestRedirectMode,
    pub(super) request_priority: Option<moli_fetch::FetchPriorityHint>,
    pub(super) request_metadata: crate::service_worker_runtime::ServiceWorkerFetchRequestMetadata,
    pub(super) policy_context: crate::types::SubresourcePolicyContext,
    pub(super) signal_id: Option<u32>,
    pub(super) load: ResourceLoadLease,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub(super) network_record: Option<PendingWorkerFetchNetworkRecord>,
    pub(super) paused_response: Option<PausedWorkerSubresourceResponse>,
    pub(super) streaming_body_source_id: Option<NetworkBodySourceId>,
}

pub(super) enum WorkerFetchEvent {
    Completion(Box<WorkerFetchCompletion>),
    StreamingStarted(WorkerFetchStreamingStarted),
    StreamingChunk(WorkerFetchStreamingChunk),
    StreamingFinished(WorkerFetchStreamingFinished),
}

pub(super) struct WorkerFetchCompletion {
    fetch_id: u32,
    network_request_headers: Option<Vec<(String, String)>>,
    result: Result<WorkerFetchResponse, String>,
}

pub(super) struct WorkerFetchStreamingStarted {
    fetch_id: u32,
    body_source_id: NetworkBodySourceId,
    head: ResponseHead,
    network_request_headers: Option<Vec<(String, String)>>,
}

pub(super) struct WorkerFetchStreamingChunk {
    body_source_id: NetworkBodySourceId,
    bytes: Vec<u8>,
}

pub(super) struct WorkerFetchStreamingFinished {
    fetch_id: u32,
    body_source_id: NetworkBodySourceId,
    head: ResponseHead,
    result: Result<SubresourceResponseBody, String>,
}

pub(super) enum WorkerFetchResponse {
    Materialized(Box<Response>),
    Streamed {
        head: Box<ResponseHead>,
        body: SubresourceResponseBody,
    },
}

impl WorkerFetchResponse {
    fn head(&self) -> ResponseHead {
        match self {
            Self::Materialized(response) => response.head(),
            Self::Streamed { head, .. } => head.as_ref().clone(),
        }
    }

    fn subresource_response_body(&self) -> SubresourceResponseBody {
        match self {
            Self::Materialized(response) => SubresourceResponseBody::from_fetch_response(response),
            Self::Streamed { body, .. } => body.clone(),
        }
    }

    fn into_fetch_parts(self) -> WorkerFetchResponseParts {
        match self {
            Self::Materialized(response) => {
                let (head, body) = response.into_body();
                WorkerFetchResponseParts::Materialized {
                    head,
                    body: Box::new(body),
                }
            }
            Self::Streamed { head, body } => {
                WorkerFetchResponseParts::Subresource { head: *head, body }
            }
        }
    }
}

enum WorkerFetchResponseParts {
    Materialized {
        head: ResponseHead,
        body: Box<ResponseBody>,
    },
    Subresource {
        head: ResponseHead,
        body: SubresourceResponseBody,
    },
}

#[derive(Clone)]
pub(super) struct PendingWorkerFetchNetworkRecord {
    pub(super) internal_id: u64,
    pub(super) url: Url,
    pub(super) method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) initial_network_request_headers: Option<Vec<(String, String)>>,
    pub(super) intercept_response: bool,
    pub(super) handle_auth_requests: bool,
}

pub(super) struct PendingWorkerXhr {
    pub(super) xhr: v8::Global<v8::Object>,
    pub(super) document_url: Url,
    pub(super) credentials_mode: RequestCredentialsMode,
    pub(super) load: ResourceLoadLease,
    pub(super) request_paused: bool,
    pub(super) request_url: Url,
    pub(super) request_method: String,
    pub(super) request_headers: Vec<(String, String)>,
    pub(super) request_body: Option<String>,
    pub(super) network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub(super) network_record: Option<PendingWorkerFetchNetworkRecord>,
    pub(super) paused_response: Option<PausedWorkerSubresourceResponse>,
}

pub(super) struct PendingWorkerCspReport {
    pub(super) load: ResourceLoadLease,
    pub(super) document_url: Url,
    pub(super) request: Request,
    pub(super) request_body: Option<String>,
    pub(super) policy_context: crate::types::SubresourcePolicyContext,
    pub(super) service_worker_runtime:
        Option<crate::service_worker_runtime::ServiceWorkerRuntimeService>,
    pub(super) service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
}

pub(super) struct PausedWorkerSubresourceResponse {
    pub(super) head: ResponseHead,
    pub(super) body: SubresourceResponseBody,
}

pub(super) struct WorkerXhrCompletion {
    pub(super) xhr_id: u32,
    pub(super) network_request_headers: Option<Vec<(String, String)>>,
    pub(super) result: Result<WorkerXhrResponse, String>,
}

pub(super) enum WorkerXhrResponse {
    Materialized(Box<Response>),
    Streamed {
        head: Box<ResponseHead>,
        body: SubresourceResponseBody,
    },
}

impl WorkerXhrResponse {
    fn head(&self) -> ResponseHead {
        match self {
            Self::Materialized(response) => response.head(),
            Self::Streamed { head, .. } => head.as_ref().clone(),
        }
    }

    fn subresource_response_body(&self) -> SubresourceResponseBody {
        match self {
            Self::Materialized(response) => SubresourceResponseBody::from_fetch_response(response),
            Self::Streamed { body, .. } => body.clone(),
        }
    }

    fn into_body_source(self) -> Result<(ResponseHead, ResponseBody), String> {
        match self {
            Self::Materialized(response) => Ok(response.into_body()),
            Self::Streamed { head, body } => body
                .materialize_bytes()
                .map(|bytes| (*head, ResponseBody::materialized_bytes(bytes)))
                .map_err(|error| format!("failed to materialize worker XHR body: {error}")),
        }
    }
}

pub(super) struct WorkerWebSocketState {
    pub(super) wrapper: v8::Global<v8::Object>,
    pub(super) command_tx: WebSocketCommandSender,
    pub(super) document_url: Url,
    pub(super) url: Url,
    pub(super) loader: crate::network::context::WorkerResourceLoader,
    pub(super) load: Option<ResourceLoadLease>,
    pub(super) opened: bool,
    pub(super) network_recorded: bool,
}

pub(super) struct PendingServiceWorkerLifecycleEvent {
    pub(super) completion: ServiceWorkerLifecycleCompletion,
    pub(super) pending_wait_until_count: usize,
    pub(super) dispatch_finished: bool,
}

pub(super) struct PendingServiceWorkerMessageEvent {
    pub(super) completion: ServiceWorkerMessageCompletion,
    pub(super) pending_wait_until_count: usize,
    pub(super) dispatch_finished: bool,
    pub(super) window_interaction_allowed: bool,
}

pub(super) struct PendingServiceWorkerNotificationEvent {
    pub(super) completion: crate::runtime::ServiceWorkerNotificationCompletion,
    pub(super) pending_wait_until_count: usize,
    pub(super) dispatch_finished: bool,
    pub(super) window_interaction_allowed: bool,
}

pub(super) struct PendingServiceWorkerPushEvent {
    pub(super) completion: crate::runtime::ServiceWorkerPushCompletion,
    pub(super) pending_wait_until_count: usize,
    pub(super) dispatch_finished: bool,
}

pub(super) struct PendingServiceWorkerSyncEvent {
    pub(super) completion: crate::runtime::ServiceWorkerSyncCompletion,
    pub(super) pending_wait_until_count: usize,
    pub(super) dispatch_finished: bool,
}

pub(super) struct PendingServiceWorkerPeriodicSyncEvent {
    pub(super) completion: crate::runtime::ServiceWorkerPeriodicSyncCompletion,
    pub(super) pending_wait_until_count: usize,
    pub(super) dispatch_finished: bool,
}

pub(super) struct PendingServiceWorkerFetchEvent {
    pub(super) completion: ServiceWorkerFetchCompletion,
    pub(super) handled_resolver: Option<v8::Global<v8::PromiseResolver>>,
    pub(super) request_signal_id: Option<u32>,
    pub(super) request_mode: RequestMode,
    pub(super) request_destination: ServiceWorkerRequestDestination,
    pub(super) pending_respond_with_response: Option<crate::network_host::MaterializedResponseHead>,
    pub(super) pending_respond_with_stream_body_source_id: Option<NetworkBodySourceId>,
    pub(super) pending_respond_with_stream_cancel_handle: Option<v8::Global<v8::Object>>,
    pub(super) respond_with_called: bool,
    pub(super) pending_respond_with: bool,
    pub(super) pending_wait_until_count: usize,
    pub(super) dispatch_finished: bool,
}

impl PendingServiceWorkerFetchEvent {
    pub(super) fn fallback(
        completion: ServiceWorkerFetchCompletion,
        request_mode: RequestMode,
        request_destination: ServiceWorkerRequestDestination,
    ) -> Self {
        Self {
            completion: ServiceWorkerFetchCompletion {
                result: ServiceWorkerFetchResult::Fallback,
                ..completion
            },
            handled_resolver: None,
            request_signal_id: None,
            request_mode,
            request_destination,
            pending_respond_with_response: None,
            pending_respond_with_stream_body_source_id: None,
            pending_respond_with_stream_cancel_handle: None,
            respond_with_called: false,
            pending_respond_with: false,
            pending_wait_until_count: 0,
            dispatch_finished: false,
        }
    }
}

pub(super) struct PendingServiceWorkerNavigationPreload {
    pub(super) owner: crate::service_worker_runtime::ServiceWorkerRunOwner,
    pub(super) _promise: v8::Global<v8::Promise>,
    pub(super) resolver: Option<v8::Global<v8::PromiseResolver>>,
    pub(super) body_source_id: Option<NetworkBodySourceId>,
}

enum WorkerImportScriptError {
    DomException { name: &'static str, message: String },
    Exception(v8::Global<v8::Value>),
}

impl WorkerImportScriptError {
    fn syntax(message: impl Into<String>) -> Self {
        Self::DomException {
            name: "SyntaxError",
            message: message.into(),
        }
    }

    fn network(message: impl Into<String>) -> Self {
        Self::DomException {
            name: "NetworkError",
            message: message.into(),
        }
    }

    fn error<'s>(scope: &mut v8::PinScope<'s, '_>, message: impl Into<String>) -> Self {
        let message = message.into();
        let value = v8_string(scope, &message)
            .map(|value| v8::Exception::error(scope, value))
            .unwrap_or_else(|| v8::Exception::error(scope, v8::String::empty(scope)));
        Self::Exception(v8::Global::new(scope, value))
    }

    fn throw(self, scope: &mut v8::PinScope<'_, '_>) {
        match self {
            Self::DomException { name, message } => {
                let exception = worker_dom_exception_value(scope, &message, name);
                scope.throw_exception(exception);
            }
            Self::Exception(value) => {
                let value = v8::Local::new(scope, value);
                scope.throw_exception(value);
            }
        }
    }
}

struct PreparedWorkerImportScript {
    final_url: Url,
    source: Option<String>,
    muted_errors: bool,
}

fn annotate_worker_exception_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
    message: Option<v8::Local<'s, v8::Message>>,
) {
    let Ok(object) = v8::Local::<v8::Object>::try_from(exception) else {
        return;
    };
    let source = message
        .and_then(|message| message.get_script_resource_name(scope))
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty());
    if let Some(source) = source
        && let Some(value) = v8_string(scope, &source)
    {
        set_worker_exception_location_if_missing(
            scope,
            object,
            WORKER_EXCEPTION_SOURCE_SLOT,
            value.into(),
        );
    }
    if let Some(line) = message.and_then(|message| message.get_line_number(scope)) {
        let value = v8::Number::new(scope, line as f64);
        set_worker_exception_location_if_missing(
            scope,
            object,
            WORKER_EXCEPTION_LINE_SLOT,
            value.into(),
        );
    }
    if let Some(column) = message.and_then(|message| message.get_start_column().checked_add(1)) {
        let value = v8::Number::new(scope, column as f64);
        set_worker_exception_location_if_missing(
            scope,
            object,
            WORKER_EXCEPTION_COLUMN_SLOT,
            value.into(),
        );
    }
}

fn set_worker_exception_location_if_missing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    if get_private_value(scope, object, slot).is_none() {
        set_private_value(scope, object, slot, value);
    }
}

/// Mutable state accessible from V8 callbacks inside the worker isolate.
pub(super) struct WorkerMessagePortWrapperEntry {
    wrapper: v8::Global<v8::Object>,
    listeners: crate::context_bootstrap::WorkerMessagePortEventListenerRegistry,
}

pub(crate) struct WorkerGlobalState {
    /// Weak handles and native cleanups owned by this worker isolate. Worker
    /// teardown clears this registry before `OwnedIsolate` is destroyed.
    pub(crate) v8_finalizers: crate::v8_finalizer::V8FinalizerRegistry,
    /// Channel to send messages back to the parent.
    pub(super) parent_tx: mpsc::UnboundedSender<WorkerToParentMessage>,
    /// Internal wake channel used by worker-owned async runtime surfaces.
    pub(crate) worker_wake_tx: mpsc::UnboundedSender<super::handle::WorkerMessage>,
    /// Shared lifecycle bit published by `Worker.terminate()` before V8 is
    /// interrupted. Worker-side task delivery must not rely on cross-thread
    /// timing between that interrupt and already-selected work.
    pub(crate) termination_requested: Arc<AtomicBool>,
    /// Whether `close()` has been called.
    pub(super) closed: bool,
    /// Timer id counter.
    pub(super) next_timer_id: u32,
    /// Inside-settings resource authority for every request owned by this
    /// WorkerGlobalScope. Even data/blob workers retain the creator's browser
    /// backend so later fetch/XHR/module/WebSocket work has an exact owner.
    pub(super) loader: crate::network::context::WorkerResourceLoader,
    /// Whether this runtime is a dedicated or shared worker global.
    pub(super) global_kind: super::thread::WorkerGlobalKind,
    /// Whether this worker was constructed as a classic or module worker.
    pub(super) script_kind: super::thread::WorkerScriptKind,
    /// Base URL used to resolve relative worker script fetches.
    pub(super) current_script_url: Option<Url>,
    /// Referrer policy parsed from the top-level worker script response.
    pub(super) referrer_policy: Option<String>,
    /// CSP policies from outside settings used for module static imports.
    pub(super) module_static_import_content_security_policies: Vec<String>,
    /// Enforce CSP policies parsed from the top-level worker script response.
    pub(super) content_security_policies: Vec<String>,
    /// Report-only CSP policies parsed from the top-level worker script response.
    pub(super) content_security_report_only_policies: Vec<String>,
    /// Reporting API endpoints parsed from the top-level worker script response.
    pub(super) content_security_reporting_endpoints: ContentSecurityPolicyReportingEndpoints,
    /// Whether this worker global is a secure context for `[SecureContext]` APIs.
    pub(super) secure_context: bool,
    /// Network.setExtraHTTPHeaders headers inherited from the owning page/CDP session.
    pub(super) extra_http_headers: Vec<(String, String)>,
    /// Permission overrides inherited from the owning page/CDP session.
    pub(super) permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    /// Network.emulateNetworkConditions offline state inherited from the owning page/CDP session.
    pub(super) network_offline: bool,
    /// Network.setBlockedURLs patterns inherited from the owning page/CDP session.
    pub(super) blocked_url_patterns: Vec<String>,
    /// Current document network/cache partition key inherited by worker-owned requests.
    pub(super) network_partition_key: Option<String>,
    /// COEP/DIP policy container state inherited by worker-owned subresource requests.
    pub(super) policy_context: crate::types::SubresourcePolicyContext,
    /// Fetch domain subresource interception inherited from the owning page/CDP session.
    pub(super) fetch_subresource_interception_enabled: bool,
    pub(super) fetch_subresource_interception_resource_type: Option<SubresourceResourceType>,
    /// Async fetch completions routed back onto the worker event loop.
    pub(super) fetch_completion_tx: mpsc::UnboundedSender<WorkerFetchEvent>,
    /// Promise resolvers pending async fetch completion.
    pub(super) pending_fetches: HashMap<u32, PendingWorkerFetch>,
    /// Worker-local pending body sources for headers-first worker fetch.
    pub(crate) pending_network_body_sources:
        HashMap<NetworkBodySourceId, crate::network_host::PendingNetworkBodySourceState>,
    pub(crate) pending_network_body_clones: HashMap<NetworkBodySourceId, Vec<NetworkBodySourceId>>,
    /// Fetch id counter.
    pub(super) next_fetch_id: u32,
    /// Async XHR completions routed back onto the worker event loop.
    pub(super) xhr_completion_tx: mpsc::UnboundedSender<WorkerXhrCompletion>,
    /// In-flight worker XHR requests keyed by internal id.
    pub(super) pending_xhrs: HashMap<u32, PendingWorkerXhr>,
    /// Worker XHR id counter.
    pub(super) next_xhr_id: u32,
    /// Worker-owned CSP report requests paused for Fetch domain request-stage interception.
    pub(super) pending_csp_reports: HashMap<u32, PendingWorkerCspReport>,
    /// Worker-local TextDecoder state. TextEncoder is stateless, but TextDecoder
    /// can stream and needs decoder state tied to this worker's isolate.
    pub(crate) text_codecs: TextCodecStore,
    /// Worker-local AbortController/AbortSignal runtime state.
    pub(super) abort: Rc<RefCell<WorkerAbortStore>>,
    /// Worker-thread view of the creator's browser-context/partition runtime.
    pub(super) worker_context_runtime: crate::runtime::RendererWorkerContextRuntime,
    /// Browser-context Service Worker owner inherited by real worker clients.
    pub(super) service_worker_runtime:
        Option<crate::service_worker_runtime::ServiceWorkerRuntimeService>,
    /// Live Service Worker client id for DedicatedWorker/SharedWorker globals.
    pub(super) service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    /// Owner-scoped MessagePort state shared with page and nested worker contexts.
    pub(super) message_port_registry: SharedMessagePortRegistry,
    /// Worker-owned MessagePort wrappers keyed by runtime port id.
    pub(super) message_port_wrappers: HashMap<MessagePortId, WorkerMessagePortWrapperEntry>,
    /// SharedWorker connect-event ports whose lifetime is owned by the shared worker runtime.
    pub(super) shared_worker_connection_ports: HashSet<MessagePortId>,
    /// Worker-owned BroadcastChannel wrappers keyed by runtime channel id.
    pub(super) broadcast_channel_registry: SharedBroadcastChannelRegistry,
    pub(super) broadcast_channel_storage_key: MoliStorageKey,
    pub(super) storage_key: MoliStorageKey,
    pub(super) broadcast_channel_wrappers: HashMap<BroadcastChannelId, v8::Global<v8::Object>>,
    /// IndexedDB storage manager inherited from the owning browser context.
    pub(super) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    /// Storage Buckets registry inherited from the owning browser context.
    pub(super) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    /// Async WebSocket transport events routed back onto this worker loop.
    pub(super) websocket_event_tx: tokio::sync::mpsc::Sender<WebSocketEvent>,
    /// Worker-owned classic WebSocket connections keyed by socket id.
    pub(super) websockets: HashMap<u64, WorkerWebSocketState>,
    /// Worker-local WebSocket id counter.
    pub(super) next_websocket_id: u64,
    /// Worker-local dedicated workers created through `new Worker(...)`.
    pub(super) next_nested_worker_id: u64,
    pub(super) nested_worker_wrappers: HashMap<DedicatedWorkerId, v8::Global<v8::Object>>,
    /// Heavyweight WebCrypto completions routed back onto the worker event loop.
    pub(super) webcrypto_completion_tx: mpsc::UnboundedSender<WorkerWebCryptoCompletion>,
    /// In-flight worker WebCrypto blocking tasks keyed by task id.
    pub(super) pending_webcrypto: HashMap<u64, PendingWorkerWebCryptoTask>,
    /// Worker WebCrypto task id counter (never zero so 0 can mean "unset").
    pub(super) next_webcrypto_task_id: u64,
    /// OPFS completions routed from the partition-owned storage IO sequence.
    pub(super) opfs_completion_tx: mpsc::UnboundedSender<WorkerOpfsCompletion>,
    pub(super) opfs_owner_state: Option<WorkerOpfsOwnerState>,
    /// In-flight Service Worker `periodicsync` events keyed by runtime event id.
    pub(super) pending_service_worker_periodic_sync_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerPeriodicSyncEvent>,
    /// In-flight Service Worker `clients.get()` / `clients.matchAll()` queries keyed by request id.
    pub(super) pending_service_worker_client_queries: HashMap<u64, PendingServiceWorkerClientQuery>,
    /// In-flight Service Worker `WindowClient.navigate()` requests keyed by request id.
    pub(super) pending_service_worker_client_navigates:
        HashMap<u64, PendingServiceWorkerClientNavigate>,
    /// In-flight Service Worker `WindowClient.focus()` requests keyed by request id.
    pub(super) pending_service_worker_client_focuses: HashMap<u64, PendingServiceWorkerClientFocus>,
    /// In-flight Service Worker `clients.openWindow()` requests keyed by request id.
    pub(super) pending_service_worker_clients_open_windows:
        HashMap<u64, PendingServiceWorkerClientsOpenWindow>,
    /// In-flight Service Worker `registration.showNotification()` requests keyed by request id.
    pub(super) pending_service_worker_show_notifications:
        HashMap<u64, PendingServiceWorkerShowNotification>,
    /// In-flight Service Worker `registration.getNotifications()` requests keyed by request id.
    pub(super) pending_service_worker_get_notifications:
        HashMap<u64, PendingServiceWorkerGetNotifications>,
    /// In-flight Service Worker `registration.sync.register()` requests keyed by request id.
    pub(super) pending_service_worker_sync_registrations:
        HashMap<u64, PendingServiceWorkerSyncRegistration>,
    /// In-flight Service Worker `registration.sync.getTags()` requests keyed by request id.
    pub(super) pending_service_worker_sync_get_tags: HashMap<u64, PendingServiceWorkerSyncGetTags>,
    /// In-flight Service Worker `registration.periodicSync.register()` requests keyed by request id.
    pub(super) pending_service_worker_periodic_sync_registrations:
        HashMap<u64, PendingServiceWorkerPeriodicSyncRegistration>,
    /// In-flight Service Worker `registration.periodicSync.getTags()` requests keyed by request id.
    pub(super) pending_service_worker_periodic_sync_get_tags:
        HashMap<u64, PendingServiceWorkerPeriodicSyncGetTags>,
    /// In-flight Service Worker `registration.periodicSync.unregister()` requests keyed by request id.
    pub(super) pending_service_worker_periodic_sync_unregistrations:
        HashMap<u64, PendingServiceWorkerPeriodicSyncUnregistration>,
    /// In-flight Service Worker `registration.pushManager.subscribe()` requests keyed by request id.
    pub(super) pending_service_worker_push_subscriptions:
        HashMap<u64, PendingServiceWorkerPushSubscribe>,
    /// In-flight Service Worker `registration.pushManager.getSubscription()` requests keyed by request id.
    pub(super) pending_service_worker_push_get_subscriptions:
        HashMap<u64, PendingServiceWorkerPushGetSubscription>,
    /// In-flight Service Worker `PushSubscription.unsubscribe()` requests keyed by request id.
    pub(super) pending_service_worker_push_unsubscriptions:
        HashMap<u64, PendingServiceWorkerPushUnsubscribe>,
    pub(super) service_worker_client_query_request_ids: WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_client_navigate_request_ids: WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_client_focus_request_ids: WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_clients_open_window_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_show_notification_request_ids: WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_get_notifications_request_ids: WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_sync_registration_request_ids: WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_sync_get_tags_request_ids: WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_periodic_sync_registration_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_periodic_sync_get_tags_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_periodic_sync_unregistration_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_push_subscription_request_ids: WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_push_get_subscription_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    pub(super) service_worker_push_unsubscription_request_ids:
        WorkerServiceWorkerRequestIdAllocator,
    /// Count of active Service Worker events that allow window interaction.
    pub(super) service_worker_window_interaction_allowed_count: usize,
    /// Service Worker lifecycle events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(super) pending_service_worker_lifecycle_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerLifecycleEvent>,
    /// Service Worker fetch events currently waiting for dispatch, `respondWith()`,
    /// and minimal `waitUntil()` promises to finish.
    pub(super) pending_service_worker_fetch_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerFetchEvent>,
    /// Navigation preload promises and response bodies stay alive until preload complete/error.
    pub(super) pending_service_worker_navigation_preloads:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerNavigationPreload>,
    /// Service Worker message events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(super) pending_service_worker_message_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerMessageEvent>,
    /// Service Worker notification events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(super) pending_service_worker_notification_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerNotificationEvent>,
    /// Service Worker push events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(super) pending_service_worker_push_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerPushEvent>,
    /// Service Worker sync events currently waiting for dispatch and
    /// `ExtendableEvent.waitUntil()` promises to finish.
    pub(super) pending_service_worker_sync_events:
        HashMap<ServiceWorkerEventId, PendingServiceWorkerSyncEvent>,
}

/// Checked allocator for one operation-local Service Worker request namespace.
///
/// The parent/worker protocol intentionally gives each operation its own
/// namespace. Keeping that shape avoids coupling unrelated APIs while making
/// exhaustion fail instead of overwriting a live pending resolver.
#[derive(Debug)]
pub(super) struct WorkerServiceWorkerRequestIdAllocator {
    next: u64,
}

impl Default for WorkerServiceWorkerRequestIdAllocator {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl WorkerServiceWorkerRequestIdAllocator {
    fn allocate(&mut self) -> u64 {
        let request_id = self.next;
        self.next = request_id
            .checked_add(1)
            .expect("worker Service Worker request id space exhausted");
        request_id
    }
}

#[cfg(test)]
mod service_worker_request_id_allocator_tests {
    use super::WorkerServiceWorkerRequestIdAllocator;

    #[test]
    #[should_panic(expected = "worker Service Worker request id space exhausted")]
    fn operation_local_request_ids_never_wrap() {
        let mut ids = WorkerServiceWorkerRequestIdAllocator { next: u64::MAX };
        let _ = ids.allocate();
    }
}

impl WorkerGlobalState {
    /// Register a pending worker WebCrypto task and return the routing tuple the
    /// blocking task needs to report completion. Runs synchronously inside the
    /// V8 callback before control returns to the worker event loop.
    pub(super) fn register_pending_webcrypto_task(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> (u64, mpsc::UnboundedSender<WorkerWebCryptoCompletion>) {
        let task_id = self.next_webcrypto_task_id;
        self.next_webcrypto_task_id = task_id
            .checked_add(1)
            .expect("worker WebCrypto task id exhausted");
        self.pending_webcrypto
            .insert(task_id, PendingWorkerWebCryptoTask { resolver });
        (task_id, self.webcrypto_completion_tx.clone())
    }

    pub(super) fn take_pending_webcrypto_task(
        &mut self,
        task_id: u64,
    ) -> Option<PendingWorkerWebCryptoTask> {
        self.pending_webcrypto.remove(&task_id)
    }

    pub(super) fn register_pending_opfs_task(
        &mut self,
        locator: moli_storage_service::StorageBucketLocator,
        handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
        settlement: crate::opfs_owner_tasks::OpfsTaskSettlement,
    ) -> (u64, mpsc::UnboundedSender<WorkerOpfsCompletion>) {
        let state = self
            .opfs_owner_state
            .get_or_insert_with(WorkerOpfsOwnerState::default);
        let task_id = state.next_task_id;
        state.next_task_id = task_id
            .checked_add(1)
            .expect("worker OPFS task id exhausted");
        state.pending_tasks.insert(
            task_id,
            PendingWorkerOpfsTask {
                locator,
                handle_access,
                settlement,
            },
        );
        (task_id, self.opfs_completion_tx.clone())
    }

    pub(super) fn take_pending_opfs_task(&mut self, task_id: u64) -> Option<PendingWorkerOpfsTask> {
        self.opfs_owner_state
            .as_mut()?
            .pending_tasks
            .remove(&task_id)
    }

    pub(super) fn register_pending_service_worker_client_query(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
        query_type: PendingServiceWorkerClientQueryType,
    ) -> u64 {
        let request_id = self.service_worker_client_query_request_ids.allocate();
        self.pending_service_worker_client_queries.insert(
            request_id,
            PendingServiceWorkerClientQuery {
                resolver,
                query_type,
            },
        );
        request_id
    }

    pub(super) fn take_pending_service_worker_client_query(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerClientQuery> {
        self.pending_service_worker_client_queries
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_client_navigate(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_client_navigate_request_ids.allocate();
        self.pending_service_worker_client_navigates
            .insert(request_id, PendingServiceWorkerClientNavigate { resolver });
        request_id
    }

    pub(super) fn take_pending_service_worker_client_navigate(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerClientNavigate> {
        self.pending_service_worker_client_navigates
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_client_focus(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_client_focus_request_ids.allocate();
        self.pending_service_worker_client_focuses
            .insert(request_id, PendingServiceWorkerClientFocus { resolver });
        request_id
    }

    pub(super) fn take_pending_service_worker_client_focus(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerClientFocus> {
        self.pending_service_worker_client_focuses
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_clients_open_window(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_clients_open_window_request_ids
            .allocate();
        self.pending_service_worker_clients_open_windows.insert(
            request_id,
            PendingServiceWorkerClientsOpenWindow { resolver },
        );
        request_id
    }

    pub(super) fn take_pending_service_worker_clients_open_window(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerClientsOpenWindow> {
        self.pending_service_worker_clients_open_windows
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_show_notification(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_show_notification_request_ids.allocate();
        self.pending_service_worker_show_notifications.insert(
            request_id,
            PendingServiceWorkerShowNotification { resolver },
        );
        request_id
    }

    pub(super) fn take_pending_service_worker_show_notification(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerShowNotification> {
        self.pending_service_worker_show_notifications
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_get_notifications(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_get_notifications_request_ids.allocate();
        self.pending_service_worker_get_notifications.insert(
            request_id,
            PendingServiceWorkerGetNotifications { resolver },
        );
        request_id
    }

    pub(super) fn take_pending_service_worker_get_notifications(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerGetNotifications> {
        self.pending_service_worker_get_notifications
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_sync_registration(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_sync_registration_request_ids.allocate();
        self.pending_service_worker_sync_registrations.insert(
            request_id,
            PendingServiceWorkerSyncRegistration { resolver },
        );
        request_id
    }

    pub(super) fn take_pending_service_worker_sync_registration(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerSyncRegistration> {
        self.pending_service_worker_sync_registrations
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_sync_get_tags(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_sync_get_tags_request_ids.allocate();
        self.pending_service_worker_sync_get_tags
            .insert(request_id, PendingServiceWorkerSyncGetTags { resolver });
        request_id
    }

    pub(super) fn take_pending_service_worker_sync_get_tags(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerSyncGetTags> {
        self.pending_service_worker_sync_get_tags
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_periodic_sync_registration(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_periodic_sync_registration_request_ids
            .allocate();
        self.pending_service_worker_periodic_sync_registrations
            .insert(
                request_id,
                PendingServiceWorkerPeriodicSyncRegistration { resolver },
            );
        request_id
    }

    pub(super) fn take_pending_service_worker_periodic_sync_registration(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPeriodicSyncRegistration> {
        self.pending_service_worker_periodic_sync_registrations
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_periodic_sync_get_tags(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_periodic_sync_get_tags_request_ids
            .allocate();
        self.pending_service_worker_periodic_sync_get_tags.insert(
            request_id,
            PendingServiceWorkerPeriodicSyncGetTags { resolver },
        );
        request_id
    }

    pub(super) fn take_pending_service_worker_periodic_sync_get_tags(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPeriodicSyncGetTags> {
        self.pending_service_worker_periodic_sync_get_tags
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_periodic_sync_unregistration(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_periodic_sync_unregistration_request_ids
            .allocate();
        self.pending_service_worker_periodic_sync_unregistrations
            .insert(
                request_id,
                PendingServiceWorkerPeriodicSyncUnregistration { resolver },
            );
        request_id
    }

    pub(super) fn take_pending_service_worker_periodic_sync_unregistration(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPeriodicSyncUnregistration> {
        self.pending_service_worker_periodic_sync_unregistrations
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_push_subscribe(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self.service_worker_push_subscription_request_ids.allocate();
        self.pending_service_worker_push_subscriptions
            .insert(request_id, PendingServiceWorkerPushSubscribe { resolver });
        request_id
    }

    pub(super) fn take_pending_service_worker_push_subscribe(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPushSubscribe> {
        self.pending_service_worker_push_subscriptions
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_push_get_subscription(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_push_get_subscription_request_ids
            .allocate();
        self.pending_service_worker_push_get_subscriptions.insert(
            request_id,
            PendingServiceWorkerPushGetSubscription { resolver },
        );
        request_id
    }

    pub(super) fn take_pending_service_worker_push_get_subscription(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPushGetSubscription> {
        self.pending_service_worker_push_get_subscriptions
            .remove(&request_id)
    }

    pub(super) fn register_pending_service_worker_push_unsubscribe(
        &mut self,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> u64 {
        let request_id = self
            .service_worker_push_unsubscription_request_ids
            .allocate();
        self.pending_service_worker_push_unsubscriptions
            .insert(request_id, PendingServiceWorkerPushUnsubscribe { resolver });
        request_id
    }

    pub(super) fn take_pending_service_worker_push_unsubscribe(
        &mut self,
        request_id: u64,
    ) -> Option<PendingServiceWorkerPushUnsubscribe> {
        self.pending_service_worker_push_unsubscriptions
            .remove(&request_id)
    }

    pub(super) fn consume_service_worker_window_interaction(&mut self) -> bool {
        if self.service_worker_window_interaction_allowed_count == 0 {
            return false;
        }

        self.service_worker_window_interaction_allowed_count = self
            .service_worker_window_interaction_allowed_count
            .saturating_sub(1);

        if let Some((_, pending)) = self
            .pending_service_worker_message_events
            .iter_mut()
            .find(|(_, pending)| pending.window_interaction_allowed)
        {
            pending.window_interaction_allowed = false;
            return true;
        }

        if let Some((_, pending)) = self
            .pending_service_worker_notification_events
            .iter_mut()
            .find(|(_, pending)| pending.window_interaction_allowed)
        {
            pending.window_interaction_allowed = false;
        }

        true
    }
}

/// Register a pending worker WebCrypto task from a callback scope.
///
/// Returns the routing tuple the blocking task needs to report completion, or
/// `None` when the current scope is not a worker global (e.g. the page runtime
/// or a bare unit-test context). This is the worker-lane analog of the page
/// `register_webcrypto_task`.
pub(crate) fn register_worker_webcrypto_task(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
) -> Option<(u64, mpsc::UnboundedSender<WorkerWebCryptoCompletion>)> {
    let state = get_worker_state(scope)?;
    let resolver = v8::Global::new(scope, resolver);
    let mut state = state.borrow_mut();
    Some(state.register_pending_webcrypto_task(resolver))
}

pub(crate) fn register_worker_opfs_task(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    locator: moli_storage_service::StorageBucketLocator,
    handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
) -> Option<(u64, mpsc::UnboundedSender<WorkerOpfsCompletion>)> {
    let state = get_worker_state(scope)?;
    let resolver = v8::Global::new(scope, resolver);
    let mut state = state.borrow_mut();
    Some(state.register_pending_opfs_task(
        locator,
        handle_access,
        crate::opfs_owner_tasks::OpfsTaskSettlement::Promise(resolver),
    ))
}

pub(crate) fn register_worker_opfs_iterator_task(
    scope: &mut v8::PinScope<'_, '_>,
    locator: moli_storage_service::StorageBucketLocator,
    registry: crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry,
    iterator_id: u32,
    keep_alive: v8::Global<v8::Object>,
    handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
) -> Option<(u64, mpsc::UnboundedSender<WorkerOpfsCompletion>)> {
    let state = get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    Some(state.register_pending_opfs_task(
        locator,
        handle_access,
        crate::opfs_owner_tasks::OpfsTaskSettlement::DirectoryIterator {
            registry,
            iterator_id,
            keep_alive,
        },
    ))
}

pub(crate) fn register_worker_opfs_move_task(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    handle: v8::Local<'_, v8::Object>,
    mutation: crate::opfs_owner_tasks::OpfsHandleMutationGuard,
    locator: moli_storage_service::StorageBucketLocator,
    handle_access: Option<crate::opfs_owner_tasks::OpfsHandleAccessContext>,
) -> Option<(u64, mpsc::UnboundedSender<WorkerOpfsCompletion>)> {
    let state = get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    Some(state.register_pending_opfs_task(
        locator,
        handle_access,
        crate::opfs_owner_tasks::OpfsTaskSettlement::Move {
            resolver: v8::Global::new(scope, resolver),
            handle: v8::Global::new(scope, handle),
            mutation,
        },
    ))
}

pub(crate) fn worker_opfs_handle_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::opfs_owner_tasks::OpfsHandleRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .opfs_owner_state
            .as_ref()?
            .handles
            .clone(),
    )
}

pub(crate) fn ensure_worker_opfs_handle_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::opfs_owner_tasks::OpfsHandleRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow_mut()
            .opfs_owner_state
            .get_or_insert_with(WorkerOpfsOwnerState::default)
            .handles
            .clone(),
    )
}

pub(crate) fn worker_opfs_directory_iterator_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .opfs_owner_state
            .as_ref()?
            .directory_iterators
            .clone(),
    )
}

pub(crate) fn ensure_worker_opfs_directory_iterator_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::opfs_owner_tasks::OpfsDirectoryIteratorRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow_mut()
            .opfs_owner_state
            .get_or_insert_with(WorkerOpfsOwnerState::default)
            .directory_iterators
            .clone(),
    )
}

pub(crate) fn cancel_worker_opfs_task(scope: &mut v8::PinScope<'_, '_>, task_id: u64) {
    if let Some(state) = get_worker_state(scope) {
        state.borrow_mut().take_pending_opfs_task(task_id);
    }
}

/// Settle a worker WebCrypto promise once its blocking task reports back on the
/// worker event loop. Terminating a worker drops this state and its completion
/// receiver together, so a separate reset generation is unnecessary.
pub(super) fn drain_worker_webcrypto_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerWebCryptoCompletion,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_webcrypto_task(completion.task_id) else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    crate::script_vm::webcrypto_tasks::settle_webcrypto_task_result(
        scope,
        resolver,
        completion.result,
    );
}

pub(super) fn drain_worker_opfs_completion(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    completion: WorkerOpfsCompletion,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_opfs_task(completion.task_id) else {
            return;
        };
        pending
    };
    let handle_access = pending.handle_access;
    match pending.settlement {
        crate::opfs_owner_tasks::OpfsTaskSettlement::Promise(resolver) => {
            let resolver = v8::Local::new(scope, &resolver);
            crate::context_bootstrap::settle_opfs_task_result(
                scope,
                resolver,
                &pending.locator,
                handle_access.as_ref(),
                completion.result,
            );
        }
        crate::opfs_owner_tasks::OpfsTaskSettlement::Move {
            resolver,
            handle,
            mutation,
        } => {
            let resolver = v8::Local::new(scope, &resolver);
            let handle = v8::Local::new(scope, &handle);
            crate::context_bootstrap::settle_opfs_move_task_result(
                scope,
                resolver,
                handle,
                &pending.locator,
                handle_access.as_ref(),
                completion.result,
            );
            drop(mutation);
        }
        crate::opfs_owner_tasks::OpfsTaskSettlement::DirectoryIterator {
            registry,
            iterator_id,
            keep_alive,
        } => {
            crate::context_bootstrap::settle_opfs_directory_iterator_task_result(
                scope,
                &registry,
                iterator_id,
                &pending.locator,
                handle_access.as_ref(),
                completion.result,
            );
            drop(keep_alive);
        }
    }
}

pub(super) fn drain_service_worker_client_query_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerClientQueryResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_client_query(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match pending.query_type {
        PendingServiceWorkerClientQueryType::Get => {
            let value = result
                .clients
                .into_iter()
                .next()
                .and_then(|client| build_service_worker_client_object_from_snapshot(scope, &client))
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::undefined(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        PendingServiceWorkerClientQueryType::MatchAll => {
            let array = v8::Array::new(scope, result.clients.len() as i32);
            for (index, client) in result.clients.iter().enumerate() {
                if let Some(object) =
                    build_service_worker_client_object_from_snapshot(scope, client)
                {
                    let _ = array.set_index(scope, index as u32, object.into());
                }
            }
            let _ = resolver.resolve(scope, array.into());
        }
    }
}

pub(super) fn drain_service_worker_client_navigate_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerClientNavigateResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_client_navigate(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(Some(client)) => {
            let value = build_service_worker_client_object_from_snapshot(scope, &client)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Ok(None) => {
            let _ = resolver.resolve(scope, v8::null(scope).into());
        }
        Err(error) => match error {
            ServiceWorkerClientNavigateError::TypeError(message) => {
                let Some(message) = v8_string(scope, &message) else {
                    let _ = resolver.reject(scope, v8::undefined(scope).into());
                    return;
                };
                let error = v8::Exception::type_error(scope, message);
                let _ = resolver.reject(scope, error);
            }
        },
    }
}

pub(super) fn drain_service_worker_client_focus_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: crate::runtime::ServiceWorkerClientFocusResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_client_focus(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(client) => {
            let value = build_service_worker_client_object_from_snapshot(scope, &client)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Err(error) => {
            reject_service_worker_client_focus_error(scope, resolver, error);
        }
    }
}

fn reject_service_worker_client_focus_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    error: ServiceWorkerClientFocusError,
) {
    match error {
        ServiceWorkerClientFocusError::DomException { name, message } => {
            let reason = worker_dom_exception_value(scope, &message, name);
            let _ = resolver.reject(scope, reason);
        }
        ServiceWorkerClientFocusError::TypeError(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let reason = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, reason);
        }
    }
}

pub(super) fn drain_service_worker_clients_open_window_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerClientsOpenWindowResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_clients_open_window(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(Some(client)) => {
            let value = build_service_worker_client_object_from_snapshot(scope, &client)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Ok(None) => {
            let _ = resolver.resolve(scope, v8::null(scope).into());
        }
        Err(error) => match error {
            ServiceWorkerClientsOpenWindowError::TypeError(message) => {
                let Some(message) = v8_string(scope, &message) else {
                    let _ = resolver.reject(scope, v8::undefined(scope).into());
                    return;
                };
                let error = v8::Exception::type_error(scope, message);
                let _ = resolver.reject(scope, error);
            }
        },
    }
}

pub(super) fn drain_service_worker_show_notification_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerShowNotificationResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_show_notification(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(super) fn drain_service_worker_get_notifications_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerGetNotificationsResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_get_notifications(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    let notifications = match result.result {
        Ok(notifications) => notifications,
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
            return;
        }
    };
    let array = v8::Array::new(scope, notifications.len() as i32);
    for (index, notification) in notifications.iter().enumerate() {
        if let Some(object) =
            crate::context_bootstrap::build_notification_object_from_snapshot(scope, notification)
        {
            let _ = array.set_index(scope, index as u32, object.into());
        }
    }
    let _ = resolver.resolve(scope, array.into());
}

pub(super) fn drain_service_worker_sync_registration_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerSyncRegistrationResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_sync_registration(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(super) fn drain_service_worker_sync_get_tags_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerSyncGetTagsResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_sync_get_tags(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(tags) => {
            let array = v8::Array::new(scope, tags.len() as i32);
            for (index, tag) in tags.iter().enumerate() {
                if let Some(value) = v8_string(scope, tag) {
                    let _ = array.set_index(scope, index as u32, value.into());
                }
            }
            let _ = resolver.resolve(scope, array.into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(super) fn drain_service_worker_periodic_sync_registration_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: crate::runtime::ServiceWorkerPeriodicSyncRegistrationResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_periodic_sync_registration(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(super) fn drain_service_worker_periodic_sync_get_tags_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: crate::runtime::ServiceWorkerPeriodicSyncGetTagsResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_periodic_sync_get_tags(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(tags) => {
            let array = v8::Array::new(scope, tags.len() as i32);
            for (index, tag) in tags.iter().enumerate() {
                if let Some(value) = v8_string(scope, tag) {
                    let _ = array.set_index(scope, index as u32, value.into());
                }
            }
            let _ = resolver.resolve(scope, array.into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(super) fn drain_service_worker_periodic_sync_unregistration_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: crate::runtime::ServiceWorkerPeriodicSyncUnregistrationResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_periodic_sync_unregistration(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(super) fn drain_service_worker_push_subscribe_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerPushSubscribeResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_push_subscribe(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(subscription) => {
            let value = build_service_worker_push_subscription_object(scope, &subscription)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(super) fn drain_service_worker_push_get_subscription_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerPushGetSubscriptionResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) =
            state.take_pending_service_worker_push_get_subscription(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(Some(subscription)) => {
            let value = build_service_worker_push_subscription_object(scope, &subscription)
                .map(v8::Local::into)
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = resolver.resolve(scope, value);
        }
        Ok(None) => {
            let _ = resolver.resolve(scope, v8::null(scope).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(super) fn drain_service_worker_push_unsubscribe_result(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    result: ServiceWorkerPushUnsubscribeResult,
) {
    let pending = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.take_pending_service_worker_push_unsubscribe(result.request_id)
        else {
            return;
        };
        pending
    };
    let resolver = v8::Local::new(scope, &pending.resolver);
    match result.result {
        Ok(unsubscribed) => {
            let _ = resolver.resolve(scope, v8::Boolean::new(scope, unsubscribed).into());
        }
        Err(message) => {
            let Some(message) = v8_string(scope, &message) else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let error = v8::Exception::type_error(scope, message);
            let _ = resolver.reject(scope, error);
        }
    }
}

pub(crate) struct NestedWorkerContext {
    pub(crate) worker_id: DedicatedWorkerId,
    pub(crate) base_url: Url,
    pub(crate) loader: crate::network::context::WorkerResourceLoader,
    pub(crate) worker_context_runtime: crate::runtime::RendererWorkerContextRuntime,
    pub(crate) service_worker_runtime:
        Option<crate::service_worker_runtime::ServiceWorkerRuntimeService>,
    pub(crate) service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    pub(crate) storage_key_top_level_site: String,
    pub(crate) creator_storage_key: MoliStorageKey,
    pub(crate) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub(crate) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub(crate) module_static_import_content_security_policies: Vec<String>,
    pub(crate) require_trusted_types_for_script: bool,
    pub(crate) network_policy: super::handle::WorkerNetworkPolicy,
    pub(crate) policy_context: crate::types::SubresourcePolicyContext,
    pub(crate) wake_tx: mpsc::UnboundedSender<super::handle::WorkerMessage>,
}

pub(crate) fn reserve_nested_worker_context(
    scope: &mut v8::PinScope<'_, '_>,
    worker: v8::Local<'_, v8::Object>,
) -> Option<NestedWorkerContext> {
    let state = get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    let base_url = state.current_script_url.clone()?;
    let worker_id = DedicatedWorkerId::new(state.next_nested_worker_id);
    state.next_nested_worker_id = state
        .next_nested_worker_id
        .checked_add(1)
        .expect("nested worker id space exhausted");
    state
        .nested_worker_wrappers
        .insert(worker_id, v8::Global::new(scope, worker));
    Some(NestedWorkerContext {
        worker_id,
        base_url,
        loader: state.loader.clone(),
        worker_context_runtime: state.worker_context_runtime.clone(),
        service_worker_runtime: state.service_worker_runtime.clone(),
        service_worker_client_id: state.service_worker_client_id,
        storage_key_top_level_site: state.storage_key.top_level_site().to_owned(),
        creator_storage_key: state.storage_key.clone(),
        indexed_db_manager: state.indexed_db_manager.clone(),
        storage_bucket_store: state.storage_bucket_store.clone(),
        module_static_import_content_security_policies: state.content_security_policies.clone(),
        require_trusted_types_for_script:
            crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
                &state.content_security_policies,
            ),
        network_policy: super::handle::WorkerNetworkPolicy {
            secure_context: state.secure_context,
            permission_overrides: state.permission_overrides.clone(),
            extra_http_headers: state.extra_http_headers.clone(),
            network_offline: state.network_offline,
            blocked_url_patterns: state.blocked_url_patterns.clone(),
            network_partition_key: state.network_partition_key.clone(),
            fetch_subresource_interception_enabled: state.fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type: state
                .fetch_subresource_interception_resource_type,
        },
        policy_context: state.policy_context,
        wake_tx: state.worker_wake_tx.clone(),
    })
}

pub(crate) fn worker_service_worker_control_state(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::runtime::ServiceWorkerControlState> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let runtime = state.service_worker_runtime.as_ref()?;
    let client_id = state.service_worker_client_id?;
    runtime.matching_controller_for_client(client_id)
}

pub(super) struct NestedWorkerUnhandledError {
    pub(super) message: String,
    pub(super) filename: String,
    pub(super) lineno: u32,
    pub(super) colno: u32,
    pub(super) event_kind: super::handle::WorkerParentErrorEventKind,
}

pub(super) struct NestedWorkerDispatchResult {
    pub(super) dispatched: bool,
    pub(super) unhandled_error: Option<NestedWorkerUnhandledError>,
}

pub(crate) fn forget_nested_worker_context(
    scope: &mut v8::PinScope<'_, '_>,
    worker_id: DedicatedWorkerId,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    state.nested_worker_wrappers.remove(&worker_id).is_some()
}

pub(super) fn dispatch_nested_worker_event(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    worker_id: DedicatedWorkerId,
    message: &super::handle::WorkerToParentMessage,
) -> NestedWorkerDispatchResult {
    let Some(worker) = state
        .borrow()
        .nested_worker_wrappers
        .get(&worker_id)
        .map(|worker| v8::Local::new(scope, worker))
    else {
        return NestedWorkerDispatchResult {
            dispatched: false,
            unhandled_error: None,
        };
    };

    match message {
        super::handle::WorkerToParentMessage::Post(_) => {
            let has_message_delivery_listener =
                crate::context_bootstrap::worker_has_message_delivery_listener(scope, worker);
            let _ = crate::context_bootstrap::dispatch_worker_event(scope, worker, message);
            NestedWorkerDispatchResult {
                dispatched: has_message_delivery_listener,
                unhandled_error: None,
            }
        }
        super::handle::WorkerToParentMessage::Error {
            message: error_message,
            filename,
            lineno,
            colno,
            event_kind,
            ..
        } => {
            let unhandled = crate::context_bootstrap::dispatch_worker_event(scope, worker, message);
            NestedWorkerDispatchResult {
                dispatched: true,
                unhandled_error: unhandled.then(|| NestedWorkerUnhandledError {
                    message: error_message.clone(),
                    filename: filename.clone(),
                    lineno: *lineno,
                    colno: *colno,
                    event_kind: *event_kind,
                }),
            }
        }
        _ => NestedWorkerDispatchResult {
            dispatched: crate::context_bootstrap::dispatch_worker_event(scope, worker, message),
            unhandled_error: None,
        },
    }
}

/// Install the `DedicatedWorkerGlobalScope` APIs on the given V8 global object.
///
/// The `state_ptr` is stored as an external in V8 so callbacks can access
/// `WorkerGlobalState`.  Callers must ensure the `Rc<RefCell<WorkerGlobalState>>`
/// outlives the V8 context.
pub(super) fn install_worker_global_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: Rc<RefCell<WorkerGlobalState>>,
) -> Result<()> {
    // Store state pointer as an external on the global so callbacks can find it.
    let state_ptr = Rc::into_raw(state.clone()) as *mut c_void;
    let external = v8::External::new(scope, state_ptr);
    // Prevent leak: the Rc was consumed by into_raw, but we still hold `state`.
    // We reconstruct it so the ref-count is correct.  The raw pointer stored in
    // the external is valid as long as the Rc (held by the caller) is alive.
    unsafe { Rc::from_raw(state_ptr as *const RefCell<WorkerGlobalState>) };
    let global_kind = state.borrow().global_kind.clone();

    let (secure_context, cross_origin_isolated) = {
        let state = state.borrow();
        (
            state.secure_context,
            state.secure_context && state.policy_context.cross_origin_isolated,
        )
    };
    WorkerGlobalBootstrapPropertiesDeclaration::new(
        external,
        global,
        cross_origin_isolated,
        secure_context,
        global,
    )
    .initialize(scope, global)
    .map_err(|error| anyhow!("failed to initialize worker global bootstrap properties: {error}"))?;
    install_worker_performance(scope, global)?;
    install_worker_global_scope_constructors(scope, global, &global_kind)?;
    let realm_kind = match &global_kind {
        super::thread::WorkerGlobalKind::Dedicated { .. } => {
            crate::context_bootstrap::exposed_interfaces::RealmKind::DedicatedWorker
        }
        super::thread::WorkerGlobalKind::Shared { .. } => {
            crate::context_bootstrap::exposed_interfaces::RealmKind::SharedWorker
        }
        super::thread::WorkerGlobalKind::Service { .. } => {
            crate::context_bootstrap::exposed_interfaces::RealmKind::ServiceWorker
        }
    };
    crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
        scope,
        global,
        realm_kind,
        secure_context,
    )?;
    crate::context_bootstrap::install_trusted_types_runtime_state(scope, global)?;
    let require_trusted_types_for_script =
        crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
            &state.borrow().content_security_policies,
        );
    if require_trusted_types_for_script {
        scope
            .get_current_context()
            .set_allow_generation_from_strings(false);
        crate::context_bootstrap::install_trusted_types_eval_runtime_state(scope, global)?;
    }
    crate::context_bootstrap::install_webassembly_runtime_state(scope, global)?;
    if matches!(global_kind, super::thread::WorkerGlobalKind::Service { .. }) {
        install_service_worker_extendable_event_constructors(scope, global)?;
    }
    crate::context_bootstrap::initialize_worker_fetch_realm_state(scope, global)?;
    let subtle_crypto_available = secure_context;
    crate::context_bootstrap::initialize_worker_crypto_realm_state(
        scope,
        global,
        subtle_crypto_available,
    )?;
    crate::context_bootstrap::initialize_worker_file_realm_state(scope, global)?;
    install_worker_create_image_bitmap(scope, global)?;
    let identity = state
        .borrow()
        .loader
        .request_client()
        .browser_identity()
        .clone();
    crate::context_bootstrap::install_worker_navigator_runtime_state(
        scope,
        global,
        secure_context,
        &identity,
    )?;
    crate::context_bootstrap::install_worker_indexed_db_runtime_state(scope, global)?;
    crate::context_bootstrap::install_worker_base64_runtime_state(scope, global)?;
    install_simple_event_target_methods(scope, global, WORKER_GLOBAL_LISTENERS_SLOT, false);
    install_simple_event_target_ordered_handlers(scope, global);
    if let Some(script_url) = state.borrow().current_script_url.clone() {
        let origin = moli_url::origin_ascii_serialization(&script_url);
        WorkerGlobalOriginDeclaration::new(origin)
            .initialize(scope, global)
            .map_err(|error| anyhow!("failed to initialize worker global origin: {error}"))?;
        crate::context_bootstrap::install_worker_location_runtime_state(
            scope,
            global,
            &script_url,
        )?;
        crate::context_bootstrap::install_worker_script_url_runtime_state(
            scope,
            global,
            &script_url,
        )?;
    }

    match &global_kind {
        super::thread::WorkerGlobalKind::Dedicated { name } => {
            set_worker_global_name_prop(scope, global, name)?;
            DedicatedWorkerGlobalPostMessageDeclaration::default()
                .initialize(scope, global)
                .map_err(|error| anyhow!("failed to initialize worker postMessage: {error}"))?;
        }
        super::thread::WorkerGlobalKind::Shared { name, .. } => {
            set_worker_global_name_prop(scope, global, name)?;
        }
        super::thread::WorkerGlobalKind::Service {
            registration_id,
            version_id,
            scope_url,
        } => {
            set_worker_global_name_prop(scope, global, "")?;
            install_service_worker_global_runtime(
                scope,
                global,
                *registration_id,
                *version_id,
                scope_url,
            )?;
        }
    }

    WorkerGlobalCommonOperationsDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker global operations: {error}"))?;

    install_worker_global_event_handler_accessors(scope, global, &global_kind)?;

    // console
    install_console(scope, global)?;

    WorkerGlobalTimerOperationsDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker timers: {error}"))?;

    crate::context_bootstrap::exposed_interfaces::capture_eager_intrinsic_interfaces(
        scope, global, realm_kind,
    )?;
    Ok(())
}

fn set_worker_global_name_prop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<()> {
    // WorkerGlobalScope.name is [Replaceable] readonly in IDL. Browsers expose
    // an own global property whose assignment becomes a normal enumerable data
    // property, which WPT observes through Object.getOwnPropertyDescriptor.
    WorkerGlobalNameDeclaration::new(name.to_owned())
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker global `name`: {error}"))
}

fn install_service_worker_global_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    registration_id: crate::runtime::ServiceWorkerRegistrationId,
    version_id: crate::runtime::ServiceWorkerVersionId,
    scope_url: &Url,
) -> Result<()> {
    let registration =
        build_service_worker_global_registration(scope, registration_id, version_id, scope_url)?;
    let clients = ServiceWorkerClientsDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to build service worker clients: {error}"))?;
    ServiceWorkerGlobalRuntimeDeclaration::new(registration, clients)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize service worker global: {error}"))
}

fn build_service_worker_global_registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration_id: crate::runtime::ServiceWorkerRegistrationId,
    version_id: crate::runtime::ServiceWorkerVersionId,
    scope_url: &Url,
) -> Result<v8::Local<'s, v8::Object>> {
    let sync_manager = ServiceWorkerGlobalSyncManagerDeclaration {
        register: (),
        get_tags: (),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build service worker sync manager: {error:?}"))?;
    let periodic_sync_manager = ServiceWorkerGlobalPeriodicSyncManagerDeclaration {
        register: (),
        get_tags: (),
        unregister: (),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build service worker periodic sync manager: {error:?}"))?;
    let push_manager = ServiceWorkerGlobalPushManagerDeclaration {
        subscribe: (),
        get_subscription: (),
        permission_state: (),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build service worker push manager: {error:?}"))?;
    let navigation_preload =
        build_service_worker_global_navigation_preload_manager(scope, scope_url)?;
    let registration = ServiceWorkerGlobalRegistrationDeclaration {
        scope: scope_url.as_str().to_owned(),
        installing: (),
        waiting: (),
        active: (),
        unregister: (),
        show_notification: (),
        get_notifications: (),
        sync: sync_manager,
        periodic_sync: periodic_sync_manager,
        push_manager,
        navigation_preload,
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build service worker registration: {error:?}"))?;
    let scope_value = v8_string(scope, scope_url.as_str())
        .ok_or_else(|| anyhow!("failed to allocate service worker registration scope"))?;
    set_private_value(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_SCOPE_SLOT,
        scope_value.into(),
    );
    let registration_id_value = v8::BigInt::new_from_u64(scope, registration_id.as_u64());
    set_private_value(
        scope,
        registration,
        SERVICE_WORKER_REGISTRATION_ID_SLOT,
        registration_id_value.into(),
    );
    let version_id_value = v8::BigInt::new_from_u64(scope, version_id.as_u64());
    set_private_value(
        scope,
        registration,
        SERVICE_WORKER_VERSION_ID_SLOT,
        version_id_value.into(),
    );
    Ok(registration)
}

fn build_service_worker_global_navigation_preload_manager<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    scope_url: &Url,
) -> Result<v8::Local<'s, v8::Object>> {
    ensure_worker_interface_constructor(scope, "NavigationPreloadManager")?;
    let navigation_preload = ServiceWorkerGlobalNavigationPreloadManagerDeclaration {
        enable: (),
        disable: (),
        set_header_value: (),
        get_state: (),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build navigation preload manager: {error:?}"))?;
    let scope_value = v8_string(scope, scope_url.as_str())
        .ok_or_else(|| anyhow!("failed to allocate navigation preload registration scope"))?;
    set_private_value(
        scope,
        navigation_preload,
        SERVICE_WORKER_NAVIGATION_PRELOAD_MANAGER_SCOPE_SLOT,
        scope_value.into(),
    );
    Ok(navigation_preload)
}

enum ServiceWorkerRegistrationWorkerPhase {
    Installing,
    Waiting,
    Active,
}

fn service_worker_registration_installing_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(service_worker_registration_worker_value(
        scope,
        ServiceWorkerRegistrationWorkerPhase::Installing,
    ));
}

fn service_worker_registration_waiting_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(service_worker_registration_worker_value(
        scope,
        ServiceWorkerRegistrationWorkerPhase::Waiting,
    ));
}

fn service_worker_registration_active_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(service_worker_registration_worker_value(
        scope,
        ServiceWorkerRegistrationWorkerPhase::Active,
    ));
}

fn service_worker_registration_worker_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    phase: ServiceWorkerRegistrationWorkerPhase,
) -> v8::Local<'s, v8::Value> {
    let Some((registration_id, _, _)) = service_worker_runtime_identity(scope) else {
        return v8::null(scope).into();
    };
    let Some(snapshot) = worker_service_worker_runtime(scope)
        .and_then(|runtime| runtime.registration_snapshot_by_id(registration_id))
    else {
        return v8::null(scope).into();
    };
    let version = match phase {
        ServiceWorkerRegistrationWorkerPhase::Installing => snapshot.installing(),
        ServiceWorkerRegistrationWorkerPhase::Waiting => snapshot.waiting(),
        ServiceWorkerRegistrationWorkerPhase::Active => snapshot.active(),
    };
    version
        .and_then(|version| build_service_worker_global_service_worker(scope, version).ok())
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into())
}

pub(super) fn build_service_worker_global_service_worker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    version: &crate::service_worker_runtime::ServiceWorkerVersionSnapshot,
) -> Result<v8::Local<'s, v8::Object>> {
    ensure_worker_interface_constructor(scope, "ServiceWorker")?;
    let worker = ServiceWorkerGlobalServiceWorkerDeclaration {
        script_url: version.script_url().as_str().to_owned(),
        state: version.state().to_owned(),
        post_message: (),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to build worker ServiceWorker object: {error:?}"))?;
    let version_id_value = v8::BigInt::new_from_u64(scope, version.version_id().as_u64());
    set_private_value(
        scope,
        worker,
        SERVICE_WORKER_VERSION_ID_SLOT,
        version_id_value.into(),
    );
    install_simple_event_target_methods(scope, worker, SERVICE_WORKER_WORKER_EVENTS_SLOT, false);
    Ok(worker)
}

fn service_worker_registration_unregister_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(resolved_worker_promise(scope, v8::Boolean::new(scope, true).into()).into());
}

fn service_worker_registration_show_notification_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(title) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("ServiceWorkerRegistration.showNotification", 1),
        "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(options) = crate::context_bootstrap::notification_options_payload(scope, args.get(1))
    else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let permission_state = {
        let state = state.borrow();
        worker_permission_state(&state, "notifications")
    };
    if permission_state != "granted" {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': notification permission has not been granted.",
                ),
            ),
        );
        return;
    }
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_show_notification(v8::Global::new(scope, resolver))
    };
    if parent_tx
        .send(WorkerToParentMessage::ServiceWorkerShowNotification(
            crate::runtime::ServiceWorkerShowNotification {
                request_id,
                registration_id,
                version_id,
                title: title.0,
                tag: options.tag,
                metadata: options.metadata,
                actions: options.actions,
                data: options.data,
            },
        ))
        .is_err()
    {
        let pending = {
            let mut state = state.borrow_mut();
            state.take_pending_service_worker_show_notification(request_id)
        };
        if let Some(pending) = pending {
            let resolver = v8::Local::new(scope, &pending.resolver);
            let _ = resolver.reject(
                scope,
                v8::Exception::type_error(
                    scope,
                    v8str(
                        scope,
                        "Failed to execute 'showNotification' on 'ServiceWorkerRegistration': Service Worker runtime is unavailable.",
                    ),
                ),
            );
        }
    }
}

fn service_worker_registration_get_notifications_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = crate::context_bootstrap::notification_get_options_tag(scope, args.get(0))
    else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getNotifications' on 'ServiceWorkerRegistration': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getNotifications' on 'ServiceWorkerRegistration': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_get_notifications(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerGetNotifications(
        crate::runtime::ServiceWorkerGetNotifications {
            request_id,
            registration_id,
            version_id,
            tag,
        },
    ));
}

fn service_worker_sync_manager_register_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("SyncManager.register", 1),
        "Failed to execute 'register' on 'SyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'register' on 'SyncManager': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'register' on 'SyncManager': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let permission_state = {
        let state = state.borrow();
        service_worker_background_sync_permission_state(&state)
    };
    if permission_state != "granted" {
        let reason = worker_dom_exception_value(
            scope,
            "Background Sync permission has not been granted.",
            "NotAllowedError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_sync_registration(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerSyncRegistration(
        crate::runtime::ServiceWorkerSyncRegistration {
            request_id,
            registration_id,
            version_id,
            tag: tag.0,
        },
    ));
}

fn service_worker_sync_manager_get_tags_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getTags' on 'SyncManager': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getTags' on 'SyncManager': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_sync_get_tags(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerSyncGetTags(
        crate::runtime::ServiceWorkerSyncGetTags {
            request_id,
            registration_id,
            version_id,
        },
    ));
}

fn service_worker_periodic_sync_manager_register_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("PeriodicSyncManager.register", 1),
        "Failed to execute 'register' on 'PeriodicSyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some(options) = service_worker_periodic_sync_options(scope, args.get(1)) else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'register' on 'PeriodicSyncManager': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'register' on 'PeriodicSyncManager': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let permission_state = {
        let state = state.borrow();
        service_worker_periodic_sync_permission_state(&state)
    };
    if permission_state != "granted" {
        let reason = worker_dom_exception_value(
            scope,
            "Periodic Background Sync permission has not been granted.",
            "NotAllowedError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_periodic_sync_registration(v8::Global::new(
            scope, resolver,
        ))
    };
    let _ = parent_tx.send(
        WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(
            crate::runtime::ServiceWorkerPeriodicSyncRegistration {
                request_id,
                registration_id,
                version_id,
                tag: tag.0,
                min_interval_ms: options.min_interval,
            },
        ),
    );
}

fn service_worker_periodic_sync_manager_get_tags_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getTags' on 'PeriodicSyncManager': registration is unavailable.",
                ),
            ),
        );
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.reject(
            scope,
            v8::Exception::type_error(
                scope,
                v8str(
                    scope,
                    "Failed to execute 'getTags' on 'PeriodicSyncManager': Service Worker runtime is unavailable.",
                ),
            ),
        );
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_periodic_sync_get_tags(v8::Global::new(
            scope, resolver,
        ))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(
        crate::runtime::ServiceWorkerPeriodicSyncGetTags {
            request_id,
            registration_id,
            version_id,
        },
    ));
}

fn service_worker_periodic_sync_manager_unregister_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(tag) = webidl::required_argument::<webidl::DomString>(
        scope,
        &args,
        0,
        webidl::Context::argument("PeriodicSyncManager.unregister", 1),
        "Failed to execute 'unregister' on 'PeriodicSyncManager': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_periodic_sync_unregistration(v8::Global::new(
            scope, resolver,
        ))
    };
    let _ = parent_tx.send(
        WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(
            crate::runtime::ServiceWorkerPeriodicSyncUnregistration {
                request_id,
                registration_id,
                version_id,
                tag: tag.0,
            },
        ),
    );
}

fn service_worker_periodic_sync_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<BackgroundSyncOptions> {
    match webidl::parse_dictionary::<BackgroundSyncOptions>(
        scope,
        value,
        webidl::Context::argument("PeriodicSyncManager.register", 2),
    ) {
        Ok(options) => Some(options.unwrap_or_default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn service_worker_background_sync_permission_state(state: &WorkerGlobalState) -> String {
    if !state.secure_context {
        return "denied".to_owned();
    }
    match worker_permission_state(state, "background-sync").as_str() {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

fn service_worker_periodic_sync_permission_state(state: &WorkerGlobalState) -> String {
    if !state.secure_context {
        return "denied".to_owned();
    }
    match worker_permission_state(state, "periodic-background-sync").as_str() {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

fn service_worker_navigation_preload_manager_enable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    service_worker_navigation_preload_manager_set_enabled(scope, args, rv, true);
}

fn service_worker_navigation_preload_manager_disable_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    service_worker_navigation_preload_manager_set_enabled(scope, args, rv, false);
}

fn service_worker_navigation_preload_manager_set_enabled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    enabled: bool,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute navigation preload operation: registration scope is unavailable.",
        );
        return;
    };
    let Some(runtime) = worker_service_worker_runtime(scope) else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute navigation preload operation: Service Worker runtime is unavailable.",
        );
        return;
    };
    match runtime.set_navigation_preload_enabled_for_scope(&scope_url, enabled) {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(error) => reject_worker_navigation_preload_state_error(scope, resolver, error),
    }
}

fn service_worker_navigation_preload_manager_set_header_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let header_value = match service_worker_navigation_preload_header_value(scope, &args) {
        Ok(header_value) => header_value,
        Err(error) => {
            reject_worker_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    if !service_worker_navigation_preload_valid_header_value(&header_value) {
        reject_worker_type_error(
            scope,
            resolver,
            "The string provided to setHeaderValue is not a valid HTTP header field value.",
        );
        return;
    }
    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute 'setHeaderValue' on 'NavigationPreloadManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(runtime) = worker_service_worker_runtime(scope) else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute 'setHeaderValue' on 'NavigationPreloadManager': Service Worker runtime is unavailable.",
        );
        return;
    };
    match runtime.set_navigation_preload_header_value_for_scope(&scope_url, header_value) {
        Ok(()) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Err(error) => reject_worker_navigation_preload_state_error(scope, resolver, error),
    }
}

fn service_worker_navigation_preload_manager_get_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some(scope_url) =
        service_worker_navigation_preload_manager_scope_from_this(scope, args.this())
    else {
        reject_worker_type_error(
            scope,
            resolver,
            "Failed to execute 'getState' on 'NavigationPreloadManager': registration scope is unavailable.",
        );
        return;
    };
    let Some(runtime) = worker_service_worker_runtime(scope) else {
        reject_worker_navigation_preload_state_error(
            scope,
            resolver,
            ServiceWorkerNavigationPreloadStateError::InvalidState,
        );
        return;
    };
    let Some(state) = runtime.navigation_preload_state_for_scope(&scope_url) else {
        reject_worker_navigation_preload_state_error(
            scope,
            resolver,
            ServiceWorkerNavigationPreloadStateError::InvalidState,
        );
        return;
    };
    let state_object = build_worker_navigation_preload_state_object(scope, &state);
    let _ = resolver.resolve(scope, state_object.into());
}

fn worker_service_worker_runtime(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::service_worker_runtime::ServiceWorkerRuntimeService> {
    let state = get_worker_state(scope)?;
    state.borrow().service_worker_runtime.clone()
}

fn service_worker_navigation_preload_manager_scope_from_this<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    this: v8::Local<'s, v8::Object>,
) -> Option<Url> {
    let value = get_private_value(
        scope,
        this,
        SERVICE_WORKER_NAVIGATION_PRELOAD_MANAGER_SCOPE_SLOT,
    )?;
    let scope_string = value.to_string(scope)?.to_rust_string_lossy(scope);
    Url::parse(&scope_string).ok()
}

fn service_worker_navigation_preload_header_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<String, webidl::WebIdlError> {
    let context = webidl::Context::argument("NavigationPreloadManager.setHeaderValue", 1);
    if args.length() <= 0 {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::ByteString>(scope, args.get(0), context).map(Into::into)
}

fn service_worker_navigation_preload_valid_header_value(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch as u32 <= 0xff && !matches!(ch, '\0' | '\r' | '\n'))
}

fn build_worker_navigation_preload_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &ServiceWorkerNavigationPreloadState,
) -> v8::Local<'s, v8::Object> {
    let object = v8::Object::new(scope);
    let _ = WorkerNavigationPreloadStateDeclaration::new(state.enabled, state.header_value.clone())
        .initialize(scope, object);
    object
}

fn reject_worker_navigation_preload_state_error(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    error: ServiceWorkerNavigationPreloadStateError,
) {
    match error {
        ServiceWorkerNavigationPreloadStateError::InvalidState => {
            let reason = worker_dom_exception_value(
                scope,
                "Registration failed - no active Service Worker",
                "InvalidStateError",
            );
            let _ = resolver.reject(scope, reason);
        }
        ServiceWorkerNavigationPreloadStateError::StorageFailure => {
            reject_worker_type_error(
                scope,
                resolver,
                "Failed to persist navigation preload state.",
            );
        }
    }
}

fn reject_worker_type_error(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    message: &str,
) {
    let Some(message) = v8_string(scope, message) else {
        let _ = resolver.reject(scope, v8::undefined(scope).into());
        return;
    };
    let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
}

fn service_worker_push_manager_subscribe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    if service_worker_push_permission_state(scope) != "granted" {
        let reason = worker_dom_exception_value(
            scope,
            "Push permission has not been granted.",
            "NotAllowedError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'subscribe' on 'PushManager': registration is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'subscribe' on 'PushManager': Service Worker runtime is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let user_visible_only = service_worker_push_subscribe_user_visible_only(scope, args.get(0));
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_push_subscribe(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPushSubscribe(
        crate::runtime::ServiceWorkerPushSubscribe {
            request_id,
            registration_id,
            version_id,
            user_visible_only,
        },
    ));
}

fn service_worker_push_manager_get_subscription_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'getSubscription' on 'PushManager': registration is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'getSubscription' on 'PushManager': Service Worker runtime is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state
            .register_pending_service_worker_push_get_subscription(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPushGetSubscription(
        crate::runtime::ServiceWorkerPushGetSubscription {
            request_id,
            registration_id,
            version_id,
        },
    ));
}

fn service_worker_push_manager_permission_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let state = service_worker_push_permission_state(scope);
    let value = v8_string(scope, &state)
        .map(v8::Local::into)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.resolve(scope, value);
}

fn service_worker_push_subscribe_user_visible_only(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    options
        .get(scope, v8str(scope, "userVisibleOnly").into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn service_worker_push_permission_state(scope: &mut v8::PinScope<'_, '_>) -> String {
    let Some(state) = get_worker_state(scope) else {
        return "prompt".to_owned();
    };
    let state = state.borrow();
    if !state.secure_context {
        return "denied".to_owned();
    }
    match worker_permission_state(&state, "notifications").as_str() {
        "granted" => "granted",
        "denied" => "denied",
        _ => "prompt",
    }
    .to_owned()
}

fn build_service_worker_push_subscription_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: &ServiceWorkerPushSubscriptionSnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    let options = ServiceWorkerPushSubscriptionOptionsDeclaration::new(
        snapshot.user_visible_only,
        v8::null(scope).into(),
    )
    .bind(scope)
    .ok()?;
    ServiceWorkerPushSubscriptionDeclaration {
        endpoint: snapshot.endpoint.clone(),
        expiration_time: v8::null(scope).into(),
        options,
        unsubscribe: (),
        to_json: (),
    }
    .bind(scope)
    .ok()
}

fn service_worker_push_subscription_unsubscribe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'unsubscribe' on 'PushSubscription': registration is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'unsubscribe' on 'PushSubscription': Service Worker runtime is unavailable.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_push_unsubscribe(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPushUnsubscribe(
        crate::runtime::ServiceWorkerPushUnsubscribe {
            request_id,
            registration_id,
            version_id,
        },
    ));
}

fn service_worker_push_subscription_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let object = ObjectLiteralDeclaration::bind(scope);
    for name in ["endpoint", "expirationTime", "options"] {
        let value = args
            .this()
            .get(scope, v8str(scope, name).into())
            .unwrap_or_else(|| v8::undefined(scope).into());
        object.set_string_property(scope, name, value);
    }
    rv.set(object.into_value());
}

fn service_worker_skip_waiting_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let undefined: v8::Local<'_, v8::Value> = v8::undefined(scope).into();
        rv.set(resolved_worker_promise(scope, undefined).into());
        return;
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerSkipWaiting {
        registration_id,
        version_id,
    });
    let undefined: v8::Local<'_, v8::Value> = v8::undefined(scope).into();
    rv.set(resolved_worker_promise(scope, undefined).into());
}

fn service_worker_clients_claim_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope) {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientsClaim {
            registration_id,
            version_id,
        });
    }
    let undefined: v8::Local<'_, v8::Value> = v8::undefined(scope).into();
    rv.set(resolved_worker_promise(scope, undefined).into());
}

fn service_worker_clients_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let id = args.get(0).to_rust_string_lossy(scope);
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_client_query(
            v8::Global::new(scope, resolver),
            PendingServiceWorkerClientQueryType::Get,
        )
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientQuery(
        crate::runtime::ServiceWorkerClientQuery {
            request_id,
            registration_id,
            version_id,
            kind: crate::runtime::ServiceWorkerClientQueryKind::Get {
                exposed_client_id: id,
            },
        },
    ));
}

fn service_worker_clients_match_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let Some((registration_id, version_id, parent_tx)) = service_worker_runtime_identity(scope)
    else {
        let _ = resolver.resolve(scope, v8::Array::new(scope, 0).into());
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        let _ = resolver.resolve(scope, v8::Array::new(scope, 0).into());
        return;
    };
    let options = service_worker_client_query_options(scope, args.get(0));
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_client_query(
            v8::Global::new(scope, resolver),
            PendingServiceWorkerClientQueryType::MatchAll,
        )
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientQuery(
        crate::runtime::ServiceWorkerClientQuery {
            request_id,
            registration_id,
            version_id,
            kind: crate::runtime::ServiceWorkerClientQueryKind::MatchAll { options },
        },
    ));
}

fn service_worker_clients_open_window_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let url = args.get(0).to_rust_string_lossy(scope);
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let base_url = {
        let state = state.borrow();
        state.current_script_url.clone()
    };
    let parsed_url = base_url
        .as_ref()
        .and_then(|base_url| base_url.join(&url).ok())
        .or_else(|| Url::parse(&url).ok());
    let Some(parsed_url) = parsed_url else {
        let Some(message) = v8_string(scope, &format!("'{url}' is not a valid URL.")) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    if !service_worker_clients_open_window_scheme_can_display(&parsed_url) {
        let Some(message) = v8_string(
            scope,
            &format!("'{}' cannot be opened.", parsed_url.as_str()),
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    }
    if !state
        .borrow_mut()
        .consume_service_worker_window_interaction()
    {
        let reason = worker_dom_exception_value(
            scope,
            "Not allowed to open a window.",
            "InvalidAccessError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }

    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_clients_open_window(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientsOpenWindow(
        crate::runtime::ServiceWorkerClientsOpenWindow {
            request_id,
            source_version_id,
            url: parsed_url,
        },
    ));
}

fn service_worker_clients_open_window_scheme_can_display(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

fn service_worker_client_query_options(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> crate::runtime::ServiceWorkerClientQueryOptions {
    let default = crate::runtime::ServiceWorkerClientQueryOptions {
        include_uncontrolled: false,
        client_type: ServiceWorkerClientQueryType::Window,
    };
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return default;
    };
    let include_uncontrolled = object
        .get(scope, v8str(scope, "includeUncontrolled").into())
        .is_some_and(|value| value.boolean_value(scope));
    let client_type = object
        .get(scope, v8str(scope, "type").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .map(|value| match value.as_str() {
            "all" => ServiceWorkerClientQueryType::All,
            "worker" => ServiceWorkerClientQueryType::Worker,
            "sharedworker" => ServiceWorkerClientQueryType::SharedWorker,
            "window" => ServiceWorkerClientQueryType::Window,
            _ => ServiceWorkerClientQueryType::Window,
        })
        .unwrap_or(ServiceWorkerClientQueryType::Window);
    crate::runtime::ServiceWorkerClientQueryOptions {
        include_uncontrolled,
        client_type,
    }
}

fn resolved_worker_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Promise> {
    let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
    let _ = resolver.resolve(scope, value);
    resolver.get_promise(scope)
}

pub(crate) fn service_worker_runtime_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(
    ServiceWorkerRegistrationId,
    ServiceWorkerVersionId,
    mpsc::UnboundedSender<WorkerToParentMessage>,
)> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let WorkerGlobalKind::Service {
        registration_id,
        version_id,
        ..
    } = &state.global_kind
    else {
        return None;
    };
    Some((*registration_id, *version_id, state.parent_tx.clone()))
}

fn worker_permission_state(state: &WorkerGlobalState, permission_name: &str) -> String {
    let current_origin = state
        .current_script_url
        .as_ref()
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default();
    let mut fallback_state = None;

    for override_entry in state.permission_overrides.iter().rev() {
        let Some(name) = worker_permission_override_name(override_entry) else {
            continue;
        };
        if name != permission_name {
            continue;
        }

        if let Some(embedded_origin) = override_entry.embedded_origin.as_deref() {
            if embedded_origin == current_origin {
                return override_entry.setting.clone();
            }
            continue;
        }

        if override_entry
            .origin
            .as_deref()
            .is_some_and(|origin| origin == current_origin)
        {
            return override_entry.setting.clone();
        }

        if override_entry.origin.is_none() && fallback_state.is_none() {
            fallback_state = Some(override_entry.setting.clone());
        }
    }

    fallback_state.unwrap_or_else(|| match permission_name {
        "background-sync" | "periodic-background-sync" | "persistent-storage" => {
            "granted".to_owned()
        }
        _ => "prompt".to_owned(),
    })
}

pub(crate) fn worker_notification_permission_state(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<String> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    if !state.secure_context {
        return Some("denied".to_owned());
    }
    Some(
        match worker_permission_state(&state, "notifications").as_str() {
            "granted" => "granted",
            "denied" => "denied",
            _ => "default",
        }
        .to_owned(),
    )
}

fn worker_permission_override_name(
    override_entry: &crate::protocol_types::PermissionOverrideRegistration,
) -> Option<&str> {
    match &override_entry.permission {
        serde_json::Value::String(name) => Some(name.as_str()),
        serde_json::Value::Object(map) => map.get("name").and_then(serde_json::Value::as_str),
        _ => None,
    }
}

pub(super) fn build_service_worker_client_object_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: &ServiceWorkerClientSnapshot,
) -> Option<v8::Local<'s, v8::Object>> {
    build_service_worker_client_object(
        scope,
        client.id,
        &client.exposed_id,
        client.url.as_str(),
        client.client_type.as_webidl_str(),
        client.frame_type.as_webidl_str(),
        client.visibility_state.as_webidl_str(),
        client.focused,
    )
}

pub(super) fn build_service_worker_client_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client_id: crate::runtime::ServiceWorkerClientId,
    exposed_client_id: &str,
    url: &str,
    client_type: &'static str,
    frame_type: &'static str,
    visibility_state: &'static str,
    focused: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let url = v8_string(scope, url).unwrap_or_else(|| v8::String::empty(scope));
    let client = if client_type == "window" {
        ServiceWorkerWindowClientDeclaration {
            id: exposed_client_id.to_owned(),
            url,
            client_type,
            frame_type,
            lifecycle_state: "active",
            visibility_state,
            focused,
            post_message: (),
            focus: (),
            navigate: (),
        }
        .bind(scope)
        .ok()?
    } else {
        ServiceWorkerBaseClientDeclaration {
            id: exposed_client_id.to_owned(),
            url,
            client_type,
            post_message: (),
        }
        .bind(scope)
        .ok()?
    };
    set_private_value(
        scope,
        client,
        SERVICE_WORKER_CLIENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, client_id.as_u64()).into(),
    );
    Some(client)
}

fn service_worker_client_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    client: v8::Local<'s, v8::Object>,
) -> Option<crate::runtime::ServiceWorkerClientId> {
    let value = get_private_value(scope, client, SERVICE_WORKER_CLIENT_ID_SLOT)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, lossless) = big.u64_value();
    lossless.then(|| crate::runtime::ServiceWorkerClientId::from_u64_for_worker(id))
}

fn service_worker_version_id_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) -> Option<crate::runtime::ServiceWorkerVersionId> {
    let value = get_private_value(scope, worker, SERVICE_WORKER_VERSION_ID_SLOT)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, lossless) = big.u64_value();
    lossless.then(|| crate::runtime::ServiceWorkerVersionId::from_u64_for_binding(id))
}

fn service_worker_worker_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'ServiceWorker': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(target_version_id) = service_worker_version_id_from_object(scope, args.this()) else {
        return;
    };
    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        return;
    };
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(payload) = crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        args.get(0),
        transfer_arg,
        "ServiceWorker",
    ) else {
        return;
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerWorkerMessage(
        crate::runtime::ServiceWorkerWorkerMessage {
            source_version_id,
            target_version_id,
            payload,
        },
    ));
}

fn service_worker_runtime_message_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(
    crate::runtime::ServiceWorkerVersionId,
    mpsc::UnboundedSender<WorkerToParentMessage>,
)> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let WorkerGlobalKind::Service { version_id, .. } = &state.global_kind else {
        return None;
    };
    Some((*version_id, state.parent_tx.clone()))
}

fn service_worker_client_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'Client': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(target_client_id) = service_worker_client_id_from_object(scope, args.this()) else {
        return;
    };
    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        return;
    };
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(payload) = crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        args.get(0),
        transfer_arg,
        "Client",
    ) else {
        return;
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientMessage(
        crate::runtime::ServiceWorkerClientMessage {
            source_version_id,
            target_client_id,
            payload,
        },
    ));
}

fn service_worker_window_client_focus_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());

    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    if !state
        .borrow_mut()
        .consume_service_worker_window_interaction()
    {
        let reason = worker_dom_exception_value(
            scope,
            "Not allowed to focus a window.",
            "InvalidAccessError",
        );
        let _ = resolver.reject(scope, reason);
        return;
    }
    let Some(target_client_id) = service_worker_client_id_from_object(scope, args.this()) else {
        let Some(message) = v8_string(scope, "The client was not found.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_client_focus(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientFocus(
        crate::runtime::ServiceWorkerClientFocus {
            request_id,
            source_version_id,
            target_client_id,
        },
    ));
}

fn service_worker_window_client_navigate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    let promise = resolver.get_promise(scope);
    rv.set(promise.into());
    let url = args.get(0).to_rust_string_lossy(scope);
    let Some(target_client_id) = service_worker_client_id_from_object(scope, args.this()) else {
        let Some(message) = v8_string(scope, "The client was not found.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let Some((source_version_id, parent_tx)) = service_worker_runtime_message_identity(scope)
    else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let base_url = {
        let Some(state) = get_worker_state(scope) else {
            let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
                let _ = resolver.reject(scope, v8::undefined(scope).into());
                return;
            };
            let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
            return;
        };
        let state = state.borrow();
        state.current_script_url.clone()
    };
    let parsed_url = base_url
        .as_ref()
        .and_then(|base_url| base_url.join(&url).ok())
        .or_else(|| Url::parse(&url).ok());
    if parsed_url
        .as_ref()
        .is_none_or(|url| url.scheme() == "about")
    {
        let Some(message) = v8_string(
            scope,
            "Failed to execute 'navigate' on 'WindowClient': URL is invalid.",
        ) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    }
    let parsed_url = parsed_url.expect("checked parsed service worker client navigate URL");
    if !service_worker_window_client_can_display_url(&parsed_url) {
        let message = format!("'{}' cannot navigate.", parsed_url.as_str());
        let Some(message) = v8_string(scope, &message) else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    }
    let Some(state) = get_worker_state(scope) else {
        let Some(message) = v8_string(scope, "Service Worker runtime is unavailable.") else {
            let _ = resolver.reject(scope, v8::undefined(scope).into());
            return;
        };
        let _ = resolver.reject(scope, v8::Exception::type_error(scope, message));
        return;
    };
    let request_id = {
        let mut state = state.borrow_mut();
        state.register_pending_service_worker_client_navigate(v8::Global::new(scope, resolver))
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerClientNavigate(
        crate::runtime::ServiceWorkerClientNavigate {
            request_id,
            source_version_id,
            target_client_id,
            url: parsed_url,
        },
    ));
}

fn service_worker_window_client_can_display_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

fn install_worker_global_event_handler_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    global_kind: &super::thread::WorkerGlobalKind,
) -> Result<()> {
    if matches!(
        global_kind,
        super::thread::WorkerGlobalKind::Dedicated { .. }
    ) {
        DedicatedWorkerGlobalEventHandlersDeclaration::default().initialize(scope, global)?;
        DedicatedWorkerGlobalEventHandlerStateDeclaration::default().initialize(scope, global)?;
    }
    if matches!(global_kind, super::thread::WorkerGlobalKind::Shared { .. }) {
        SharedWorkerGlobalEventHandlersDeclaration::default().initialize(scope, global)?;
        SharedWorkerGlobalEventHandlerStateDeclaration::default().initialize(scope, global)?;
    }
    if matches!(global_kind, super::thread::WorkerGlobalKind::Service { .. }) {
        ServiceWorkerGlobalEventHandlersDeclaration::default().initialize(scope, global)?;
        ServiceWorkerGlobalEventHandlerStateDeclaration::default().initialize(scope, global)?;
    }
    WorkerGlobalCommonEventHandlersDeclaration::default().initialize(scope, global)?;
    WorkerGlobalCommonEventHandlerStateDeclaration::default().initialize(scope, global)?;
    Ok(())
}

fn worker_global_event_handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    slot_name: &str,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, global, slot_name).unwrap_or_else(|| v8::null(scope).into())
}

fn set_worker_global_event_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    slot_name: &'static str,
    event_type: Option<&str>,
    store_non_callable_objects: bool,
) {
    let stored = if value.is_function() || (store_non_callable_objects && value.is_object()) {
        value
    } else {
        v8::null(scope).into()
    };
    let active = stored.is_function();
    set_private_value(scope, global, slot_name, stored);
    if let Some(event_type) = event_type {
        simple_object_event_set_ordered_handler(
            scope,
            global,
            WORKER_GLOBAL_LISTENERS_SLOT,
            event_type,
            slot_name,
            active,
        );
    }
}

fn worker_global_onmessage_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONMESSAGE_SLOT,
    ));
}

fn worker_global_onmessage_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONMESSAGE_SLOT,
        Some("message"),
        true,
    );
}

fn worker_global_onmessageerror_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONMESSAGEERROR_SLOT,
    ));
}

fn worker_global_onmessageerror_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONMESSAGEERROR_SLOT,
        Some("messageerror"),
        true,
    );
}

fn worker_global_oninstall_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONINSTALL_SLOT,
    ));
}

fn worker_global_oninstall_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONINSTALL_SLOT,
        Some("install"),
        true,
    );
}

fn worker_global_onactivate_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONACTIVATE_SLOT,
    ));
}

fn worker_global_onactivate_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONACTIVATE_SLOT,
        Some("activate"),
        true,
    );
}

fn worker_global_onfetch_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONFETCH_SLOT,
    ));
}

fn worker_global_onfetch_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONFETCH_SLOT,
        Some("fetch"),
        true,
    );
}

fn worker_global_onpush_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONPUSH_SLOT,
    ));
}

fn worker_global_onpush_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONPUSH_SLOT,
        Some("push"),
        true,
    );
}

fn worker_global_onsync_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONSYNC_SLOT,
    ));
}

fn worker_global_onsync_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONSYNC_SLOT,
        Some("sync"),
        true,
    );
}

fn worker_global_onperiodicsync_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONPERIODICSYNC_SLOT,
    ));
}

fn worker_global_onperiodicsync_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONPERIODICSYNC_SLOT,
        Some("periodicsync"),
        true,
    );
}

fn worker_global_onnotificationclick_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONNOTIFICATIONCLICK_SLOT,
    ));
}

fn worker_global_onnotificationclick_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONNOTIFICATIONCLICK_SLOT,
        Some("notificationclick"),
        true,
    );
}

fn worker_global_onnotificationclose_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONNOTIFICATIONCLOSE_SLOT,
    ));
}

fn worker_global_onnotificationclose_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONNOTIFICATIONCLOSE_SLOT,
        Some("notificationclose"),
        true,
    );
}

fn worker_global_onerror_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONERROR_SLOT,
    ));
}

fn worker_global_onerror_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONERROR_SLOT,
        None,
        true,
    );
}

fn worker_global_onconnect_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONCONNECT_SLOT,
    ));
}

fn worker_global_onconnect_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONCONNECT_SLOT,
        Some("connect"),
        false,
    );
}

fn worker_global_onoffline_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONOFFLINE_SLOT,
    ));
}

fn worker_global_onoffline_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONOFFLINE_SLOT,
        Some("offline"),
        true,
    );
}

fn worker_global_ononline_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONONLINE_SLOT,
    ));
}

fn worker_global_ononline_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONONLINE_SLOT,
        Some("online"),
        true,
    );
}

fn worker_global_onunhandledrejection_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONUNHANDLEDREJECTION_SLOT,
    ));
}

fn worker_global_onunhandledrejection_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONUNHANDLEDREJECTION_SLOT,
        Some("unhandledrejection"),
        false,
    );
}

fn worker_global_onrejectionhandled_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(worker_global_event_handler_value(
        scope,
        args.this(),
        WORKER_GLOBAL_ONREJECTIONHANDLED_SLOT,
    ));
}

fn worker_global_onrejectionhandled_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_worker_global_event_handler(
        scope,
        args.this(),
        args.get(0),
        WORKER_GLOBAL_ONREJECTIONHANDLED_SLOT,
        Some("rejectionhandled"),
        false,
    );
}

fn install_worker_create_image_bitmap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    WorkerGlobalCreateImageBitmapDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize createImageBitmap: {error}"))
}

fn worker_create_image_bitmap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let reason = worker_dom_exception_value(
        scope,
        "The source image could not be decoded.",
        "InvalidStateError",
    );
    let _ = resolver.reject(scope, reason);
    rv.set(promise.into());
}

fn install_worker_global_scope_constructors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    global_kind: &super::thread::WorkerGlobalKind,
) -> Result<()> {
    let worker_ctor = worker_scope_constructor(scope, "WorkerGlobalScope")?;
    let worker_proto = constructor_prototype(scope, worker_ctor, "WorkerGlobalScope")?;
    set_worker_to_string_tag(scope, worker_proto, "WorkerGlobalScope");
    WorkerGlobalScopeConstructorGlobalDeclaration::new(worker_ctor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize WorkerGlobalScope global: {error}"))?;
    match global_kind {
        super::thread::WorkerGlobalKind::Dedicated { .. } => {
            install_dedicated_worker_global_constructor(scope, global, worker_ctor, worker_proto)?
        }
        super::thread::WorkerGlobalKind::Shared { .. } => {
            install_shared_worker_global_constructor(scope, global, worker_ctor, worker_proto)?
        }
        super::thread::WorkerGlobalKind::Service { .. } => {
            install_service_worker_global_constructor(scope, global, worker_ctor, worker_proto)?
        }
    }
    Ok(())
}

fn install_dedicated_worker_global_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    worker_ctor: v8::Local<'s, v8::Function>,
    worker_proto: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let dedicated_ctor = worker_scope_constructor(scope, "DedicatedWorkerGlobalScope")?;
    let dedicated_proto =
        constructor_prototype(scope, dedicated_ctor, "DedicatedWorkerGlobalScope")?;
    let _ = dedicated_proto.set_prototype(scope, worker_proto.into());
    let _ = dedicated_ctor.set_prototype(scope, worker_ctor.into());
    set_worker_to_string_tag(scope, dedicated_proto, "DedicatedWorkerGlobalScope");
    DedicatedWorkerGlobalMethodsDeclaration::default()
        .initialize(scope, dedicated_proto)
        .map_err(|error| {
            anyhow!("failed to initialize DedicatedWorkerGlobalScope methods: {error}")
        })?;
    DedicatedWorkerGlobalScopeConstructorGlobalDeclaration::new(dedicated_ctor)
        .initialize(scope, global)
        .map_err(|error| {
            anyhow!("failed to initialize DedicatedWorkerGlobalScope global: {error}")
        })?;
    let _ = global.set_prototype(scope, dedicated_proto.into());
    Ok(())
}

fn install_shared_worker_global_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    worker_ctor: v8::Local<'s, v8::Function>,
    worker_proto: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let shared_ctor = worker_scope_constructor(scope, "SharedWorkerGlobalScope")?;
    let shared_proto = constructor_prototype(scope, shared_ctor, "SharedWorkerGlobalScope")?;
    let _ = shared_proto.set_prototype(scope, worker_proto.into());
    let _ = shared_ctor.set_prototype(scope, worker_ctor.into());
    set_worker_to_string_tag(scope, shared_proto, "SharedWorkerGlobalScope");
    SharedWorkerGlobalMethodsDeclaration::default()
        .initialize(scope, shared_proto)
        .map_err(|error| {
            anyhow!("failed to initialize SharedWorkerGlobalScope methods: {error}")
        })?;
    SharedWorkerGlobalScopeConstructorGlobalDeclaration::new(shared_ctor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize SharedWorkerGlobalScope global: {error}"))?;
    let _ = global.set_prototype(scope, shared_proto.into());
    Ok(())
}

fn install_service_worker_global_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    worker_ctor: v8::Local<'s, v8::Function>,
    worker_proto: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let service_ctor = worker_scope_constructor(scope, "ServiceWorkerGlobalScope")?;
    let service_proto = constructor_prototype(scope, service_ctor, "ServiceWorkerGlobalScope")?;
    let _ = service_proto.set_prototype(scope, worker_proto.into());
    let _ = service_ctor.set_prototype(scope, worker_ctor.into());
    set_worker_to_string_tag(scope, service_proto, "ServiceWorkerGlobalScope");
    ServiceWorkerGlobalScopeConstructorGlobalDeclaration::new(service_ctor)
        .initialize(scope, global)
        .map_err(|error| {
            anyhow!("failed to initialize ServiceWorkerGlobalScope global: {error}")
        })?;
    ensure_worker_interface_constructor(scope, "NavigationPreloadManager")?;
    let _ = global.set_prototype(scope, service_proto.into());
    Ok(())
}

fn install_service_worker_extendable_event_constructors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let extendable_template = ExtendableEventTemplateDeclaration::build(scope);
    let extendable_ctor = extendable_template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build ExtendableEvent constructor"))?;
    let extendable_proto = constructor_prototype(scope, extendable_ctor, "ExtendableEvent")?;
    set_worker_to_string_tag(scope, extendable_proto, "ExtendableEvent");
    if let Some(event_ctor) = global_constructor_object(scope, "Event") {
        let _ = extendable_ctor.set_prototype(scope, event_ctor.into());
    }
    if let Some(event_proto) = global_constructor_prototype(scope, "Event") {
        let _ = extendable_proto.set_prototype(scope, event_proto.into());
    }
    ExtendableEventConstructorGlobalDeclaration::new(extendable_ctor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize ExtendableEvent global: {error}"))?;

    let message_template = ExtendableMessageEventTemplateDeclaration::build(scope);
    let message_ctor = message_template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build ExtendableMessageEvent constructor"))?;
    let message_proto = constructor_prototype(scope, message_ctor, "ExtendableMessageEvent")?;
    set_worker_to_string_tag(scope, message_proto, "ExtendableMessageEvent");
    let _ = message_ctor.set_prototype(scope, extendable_ctor.into());
    let _ = message_proto.set_prototype(scope, extendable_proto.into());
    ExtendableMessageEventConstructorGlobalDeclaration::new(message_ctor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize ExtendableMessageEvent global: {error}"))?;

    ensure_worker_interface_constructor(scope, "ServiceWorker")?;
    ensure_worker_interface_constructor(scope, "Client")?;
    ensure_worker_interface_constructor(scope, "WindowClient")?;
    if let Some(client_ctor) = global_constructor_object(scope, "Client")
        && let Some(window_ctor) = global_constructor_object(scope, "WindowClient")
    {
        let _ = window_ctor.set_prototype(scope, client_ctor.into());
    }
    if let Some(client_proto) = global_constructor_prototype(scope, "Client")
        && let Some(window_proto) = global_constructor_prototype(scope, "WindowClient")
    {
        let _ = window_proto.set_prototype(scope, client_proto.into());
    }

    Ok(())
}

fn extendable_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableEvent': Please use the 'new' operator.",
        );
        return;
    }
    let Some(event_type) = extendable_event_type_argument(scope, &args, "ExtendableEvent") else {
        return;
    };
    let Some((bubbles, cancelable, composed)) =
        extendable_event_init_flags(scope, &args, "ExtendableEvent")
    else {
        return;
    };
    initialize_extendable_event_object(
        scope,
        args.this(),
        &event_type,
        bubbles,
        cancelable,
        composed,
    );
    rv.set(args.this().into());
}

fn extendable_message_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': Please use the 'new' operator.",
        );
        return;
    }
    let Some(event_type) = extendable_event_type_argument(scope, &args, "ExtendableMessageEvent")
    else {
        return;
    };
    let init = extendable_event_init_object(&args);
    let Some((bubbles, cancelable, composed)) =
        extendable_event_init_flags(scope, &args, "ExtendableMessageEvent")
    else {
        return;
    };
    let Some(data) = extendable_message_event_data(scope, init) else {
        return;
    };
    let Some(origin) = extendable_message_event_string_member(scope, init, "origin", "") else {
        return;
    };
    let Some(last_event_id) =
        extendable_message_event_string_member(scope, init, "lastEventId", "")
    else {
        return;
    };
    let Some(source) = extendable_message_event_source(scope, init) else {
        return;
    };
    let Some(ports) = extendable_message_event_ports(scope, init) else {
        return;
    };

    let event = args.this();
    initialize_extendable_event_object(scope, event, &event_type, bubbles, cancelable, composed);
    let _ = ExtendableMessageEventStateDeclaration::new(data, origin, last_event_id, source, ports)
        .initialize(scope, event);
    rv.set(event.into());
}

fn extendable_event_type_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor_name: &'static str,
) -> Option<String> {
    if args.length() == 0 {
        throw_type_error(
            scope,
            &format!("Failed to construct '{constructor_name}': 1 argument required."),
        );
        return None;
    }
    webidl::argument::<webidl::DomString>(
        scope,
        args,
        0,
        webidl::Context::argument(constructor_name, 1),
    )
    .map(Into::into)
    .map_or_else(
        |error| {
            webidl::throw_error(scope, &error);
            None
        },
        Some,
    )
}

fn extendable_event_init_object<'s>(
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    if args.length() <= 1 {
        return None;
    }
    let value = args.get(1);
    if value.is_null_or_undefined() || !value.is_object() {
        None
    } else {
        v8::Local::<v8::Object>::try_from(value).ok()
    }
}

fn extendable_event_init_flags<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor_name: &'static str,
) -> Option<(bool, bool, bool)> {
    let init = extendable_event_init_object(args);
    Some((
        extendable_event_bool_member(scope, init, constructor_name, "bubbles")?,
        extendable_event_bool_member(scope, init, constructor_name, "cancelable")?,
        extendable_event_bool_member(scope, init, constructor_name, "composed")?,
    ))
}

fn extendable_event_bool_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    prefix: &'static str,
    key: &'static str,
) -> Option<bool> {
    let Some(init) = init else {
        return Some(false);
    };
    let value =
        match webidl::property_result(scope, init, key, webidl::Context::member(prefix, key)) {
            Ok(Some(value)) if !value.is_undefined() => value,
            Ok(_) => return Some(false),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
    webidl::convert::<webidl::Boolean>(scope, value, webidl::Context::member(prefix, key))
        .map(Into::into)
        .map_or_else(
            |error| {
                webidl::throw_error(scope, &error);
                None
            },
            Some,
        )
}

fn initialize_extendable_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
) {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let null_value: v8::Local<'_, v8::Value> = v8::null(scope).into();
    let _ = InitializedExtendableEventStateDeclaration::new(
        event_type.to_owned(),
        bubbles,
        cancelable,
        composed,
        false,
        null_value,
        null_value,
        0,
        false,
        timestamp_ms,
    )
    .initialize(scope, event);
}

fn extendable_message_event_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(init) = init else {
        return Some(v8::null(scope).into());
    };
    match webidl::property_result(
        scope,
        init,
        "data",
        webidl::Context::member("ExtendableMessageEvent", "data"),
    ) {
        Ok(Some(value)) if !value.is_undefined() => Some(value),
        Ok(_) => Some(v8::null(scope).into()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn extendable_message_event_string_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    key: &'static str,
    default: &'static str,
) -> Option<String> {
    let Some(init) = init else {
        return Some(default.to_owned());
    };
    let value = match webidl::property_result(
        scope,
        init,
        key,
        webidl::Context::member("ExtendableMessageEvent", key),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(default.to_owned()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("ExtendableMessageEvent", key),
    )
    .map(Into::into)
    .map_or_else(
        |error| {
            webidl::throw_error(scope, &error);
            None
        },
        Some,
    )
}

fn extendable_message_event_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(init) = init else {
        return Some(v8::null(scope).into());
    };
    let value = match webidl::property_result(
        scope,
        init,
        "source",
        webidl::Context::member("ExtendableMessageEvent", "source"),
    ) {
        Ok(Some(value)) if !value.is_null_or_undefined() => value,
        Ok(_) => return Some(v8::null(scope).into()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': member source is not of type Client, ServiceWorker, or MessagePort.",
        );
        return None;
    };
    if extendable_message_event_source_object_is_valid(scope, object) {
        Some(value)
    } else {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': member source is not of type Client, ServiceWorker, or MessagePort.",
        );
        None
    }
}

fn extendable_message_event_source_object_is_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    crate::context_bootstrap::message_port_id_from_object(scope, object).is_some()
        || get_private_value(scope, object, SERVICE_WORKER_CLIENT_ID_SLOT).is_some()
        || (get_private_value(scope, object, SERVICE_WORKER_VERSION_ID_SLOT).is_some()
            && object
                .get(scope, v8str(scope, "scriptURL").into())
                .is_some_and(|value| value.is_string()))
}

fn extendable_message_event_ports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Array>> {
    let Some(init) = init else {
        return Some(frozen_empty_worker_array(scope));
    };
    let value = match webidl::property_result(
        scope,
        init,
        "ports",
        webidl::Context::member("ExtendableMessageEvent", "ports"),
    ) {
        Ok(Some(value)) if !value.is_undefined() => value,
        Ok(_) => return Some(frozen_empty_worker_array(scope)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Ok(source) = v8::Local::<v8::Array>::try_from(value) else {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': member ports is not a sequence<MessagePort>.",
        );
        return None;
    };
    let ports = v8::Array::new(scope, source.length() as i32);
    for index in 0..source.length() {
        let port = source.get_index(scope, index)?;
        let Ok(port_object) = v8::Local::<v8::Object>::try_from(port) else {
            throw_type_error(
                scope,
                "Failed to construct 'ExtendableMessageEvent': member ports contains a non-MessagePort value.",
            );
            return None;
        };
        if crate::context_bootstrap::message_port_id_from_object(scope, port_object).is_none() {
            throw_type_error(
                scope,
                "Failed to construct 'ExtendableMessageEvent': member ports contains a non-MessagePort value.",
            );
            return None;
        }
        let _ = ports.set_index(scope, index, port);
    }
    let _ = ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    Some(ports)
}

fn frozen_empty_worker_array<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, 0);
    let _ = array.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    array
}

fn extendable_event_wait_until_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let exception = worker_dom_exception_value(
        scope,
        "ExtendableEvent.waitUntil() was called outside an active event dispatch.",
        "InvalidStateError",
    );
    scope.throw_exception(exception);
}

fn ensure_worker_interface_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &'static str,
) -> Result<()> {
    let global = scope.get_current_context().global(scope);
    if global
        .get(scope, v8str(scope, name).into())
        .is_some_and(|value| !value.is_undefined())
    {
        return Ok(());
    }
    let constructor = worker_scope_constructor(scope, name)?;
    let prototype = constructor_prototype(scope, constructor, name)?;
    set_worker_to_string_tag(scope, prototype, name);
    set_prop(scope, global, name, constructor.into())
}

fn worker_scope_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Function>> {
    let constructor = v8::Function::builder(worker_global_scope_constructor_callback)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build {name} constructor"))?;
    constructor.set_name(v8str(scope, name));
    Ok(constructor)
}

fn constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Object>> {
    let prototype_key = v8str(scope, "prototype");
    let prototype = constructor
        .get(scope, prototype_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("{name} constructor prototype missing"))?;
    WorkerScopePrototypeConstructorDeclaration::new(constructor)
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize {name}.prototype.constructor: {error}"))?;
    Ok(prototype)
}

fn set_worker_to_string_tag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    tag: &'static str,
) {
    let _ = WorkerPrototypeTagDeclaration::new(tag).initialize(scope, prototype);
}

fn worker_global_scope_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(scope, "Illegal constructor.");
}

/// Retrieve the `WorkerGlobalState` from a callback scope.
pub(crate) fn get_worker_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<Rc<RefCell<WorkerGlobalState>>> {
    let global = scope.get_current_context().global(scope);
    let val = get_private_value(scope, global, WORKER_STATE_SLOT)?;
    let external = v8::Local::<v8::External>::try_from(val).ok()?;
    let ptr = external.value() as *const RefCell<WorkerGlobalState>;
    // Safety: the Rc is kept alive by the worker thread for the lifetime of
    // the context.  We increment the ref-count here so we get our own Rc.
    unsafe {
        Rc::increment_strong_count(ptr);
        Some(Rc::from_raw(ptr))
    }
}

pub(super) fn service_worker_fetch_handler_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> WorkerFetchHandlerType {
    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        "fetch",
    );
    if listeners.is_empty() {
        return WorkerFetchHandlerType::NoHandler;
    }
    if listeners.iter().all(|listener| {
        listener
            .callable_function()
            .is_some_and(|callback| function_source_has_empty_body(scope, callback))
    }) {
        WorkerFetchHandlerType::EmptyFetchHandler
    } else {
        WorkerFetchHandlerType::NotSkippable
    }
}

fn function_source_has_empty_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(source) = function.to_string(scope) else {
        return false;
    };
    let normalized = source
        .to_rust_string_lossy(scope)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let Some(open_brace) = normalized.find('{') else {
        return false;
    };
    let Some(close_brace) = normalized.rfind('}') else {
        return false;
    };
    open_brace < close_brace && normalized[open_brace + 1..close_brace].is_empty()
}

pub(crate) fn worker_message_port_wake_sender(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<mpsc::UnboundedSender<super::handle::WorkerMessage>> {
    Some(get_worker_state(scope)?.borrow().worker_wake_tx.clone())
}

pub(crate) fn worker_message_port_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<SharedMessagePortRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .message_port_registry
            .clone(),
    )
}

pub(crate) fn worker_broadcast_channel_wake_sender(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<mpsc::UnboundedSender<super::handle::WorkerMessage>> {
    Some(get_worker_state(scope)?.borrow().worker_wake_tx.clone())
}

pub(crate) fn worker_broadcast_channel_registry(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<SharedBroadcastChannelRegistry> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .broadcast_channel_registry
            .clone(),
    )
}

pub(crate) fn worker_broadcast_channel_storage_key(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<MoliStorageKey> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .broadcast_channel_storage_key
            .clone(),
    )
}

pub(crate) fn worker_global_is_closed(scope: &mut v8::PinScope<'_, '_>) -> bool {
    get_worker_state(scope).is_some_and(|state| state.borrow().closed)
}

pub(crate) fn worker_termination_requested(scope: &mut v8::PinScope<'_, '_>) -> bool {
    get_worker_state(scope)
        .is_some_and(|state| state.borrow().termination_requested.load(Ordering::Acquire))
}

pub(crate) fn worker_storage_key(scope: &mut v8::PinScope<'_, '_>) -> Option<MoliStorageKey> {
    Some(get_worker_state(scope)?.borrow().storage_key.clone())
}

pub(crate) fn worker_storage_partition_identity(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::runtime::RendererStoragePartitionIdentity> {
    Some(
        get_worker_state(scope)?
            .borrow()
            .worker_context_runtime
            .storage_partition_identity(),
    )
}

pub(crate) fn worker_current_script_url(scope: &mut v8::PinScope<'_, '_>) -> Option<Url> {
    get_worker_state(scope)?.borrow().current_script_url.clone()
}

pub(crate) fn worker_allows_trusted_type_policy_name(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
) -> Option<bool> {
    Some(
        crate::content_security_policy::content_security_policy_allows_trusted_type_policy_name(
            &get_worker_state(scope)?.borrow().content_security_policies,
            name,
        ),
    )
}

pub(crate) fn worker_allows_trusted_types_eval(scope: &mut v8::PinScope<'_, '_>) -> Option<bool> {
    Some(
        crate::content_security_policy::content_security_policy_allows_trusted_types_eval(
            &get_worker_state(scope)?.borrow().content_security_policies,
        ),
    )
}

pub(crate) fn dispatch_worker_trusted_types_sink_violation_event(
    scope: &mut v8::PinScope<'_, '_>,
    sink: &str,
    sample: &str,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    content_security_policy::dispatch_worker_trusted_types_sink_violation_event_for_state(
        scope, &state, sink, sample,
    );
}

pub(crate) fn worker_exception_report_target(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<(mpsc::UnboundedSender<WorkerToParentMessage>, String)> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let script_url = state
        .current_script_url
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    Some((state.parent_tx.clone(), script_url))
}

pub(crate) fn worker_uses_shared_worker_agent_cluster(scope: &mut v8::PinScope<'_, '_>) -> bool {
    get_worker_state(scope).is_some_and(|state| {
        matches!(
            state.borrow().global_kind,
            super::thread::WorkerGlobalKind::Shared { .. }
        )
    })
}

pub(crate) fn register_worker_message_port_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    port: v8::Local<'_, v8::Object>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let (abort, retired_listener_ids) = {
        let mut state = state.borrow_mut();
        let abort = state.abort.clone();
        let retired_listener_ids = state
            .message_port_wrappers
            .insert(
                port_id,
                WorkerMessagePortWrapperEntry {
                    wrapper: v8::Global::new(scope, port),
                    listeners:
                        crate::context_bootstrap::WorkerMessagePortEventListenerRegistry::default(),
                },
            )
            .map(|mut previous| previous.listeners.take_listener_ids())
            .unwrap_or_default();
        (abort, retired_listener_ids)
    };
    for listener_id in retired_listener_ids {
        abort
            .borrow_mut()
            .unregister_message_port_listener(port_id, listener_id);
    }
}

pub(crate) fn register_shared_worker_connection_port(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    state
        .borrow_mut()
        .shared_worker_connection_ports
        .insert(port_id);
}

pub(crate) fn forget_worker_message_port_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let (abort, retired_listener_ids) = {
        let mut state = state.borrow_mut();
        let abort = state.abort.clone();
        let retired_listener_ids = state
            .message_port_wrappers
            .remove(&port_id)
            .map(|mut entry| entry.listeners.take_listener_ids())
            .unwrap_or_default();
        (abort, retired_listener_ids)
    };
    for listener_id in retired_listener_ids {
        abort
            .borrow_mut()
            .unregister_message_port_listener(port_id, listener_id);
    }
}

pub(crate) fn worker_message_port_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    state
        .message_port_wrappers
        .get(&port_id)
        .map(|entry| v8::Local::new(scope, &entry.wrapper))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn register_worker_message_port_event_listener(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    event_type: String,
    order: f64,
    callback: WebIdlCallbackInterface,
    options: crate::webidl::EventListenerOptions,
) -> Option<crate::context_bootstrap::MessagePortEventListenerId> {
    let state = get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    let entry = state.message_port_wrappers.get_mut(&port_id)?;
    entry.listeners.register(
        scope,
        event_type,
        order,
        callback,
        options.capture,
        options.once,
        options.passive.unwrap_or(false),
    )
}

pub(crate) fn remove_worker_message_port_event_listener(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    event_type: &str,
    callback: &WebIdlCallbackInterface,
    capture: bool,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let (abort, listener_id) = {
        let mut state = state.borrow_mut();
        let abort = state.abort.clone();
        let listener_id = state
            .message_port_wrappers
            .get_mut(&port_id)
            .and_then(|entry| {
                entry
                    .listeners
                    .remove_matching(scope, event_type, callback, capture)
            });
        (abort, listener_id)
    };
    let Some(listener_id) = listener_id else {
        return false;
    };
    abort
        .borrow_mut()
        .unregister_message_port_listener(port_id, listener_id);
    true
}

pub(crate) fn remove_worker_message_port_event_listener_by_id(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    listener_id: crate::context_bootstrap::MessagePortEventListenerId,
) -> bool {
    let Some(state) = get_worker_state(scope) else {
        return false;
    };
    let (abort, removed) = {
        let mut state = state.borrow_mut();
        let abort = state.abort.clone();
        let removed = state
            .message_port_wrappers
            .get_mut(&port_id)
            .is_some_and(|entry| entry.listeners.remove_listener_id(listener_id));
        (abort, removed)
    };
    if removed {
        abort
            .borrow_mut()
            .unregister_message_port_listener(port_id, listener_id);
    }
    removed
}

pub(crate) fn worker_message_port_event_listener_snapshots(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    event_type: &str,
) -> Vec<crate::context_bootstrap::MessagePortEventListenerSnapshot> {
    let Some(state) = get_worker_state(scope) else {
        return Vec::new();
    };
    state
        .borrow()
        .message_port_wrappers
        .get(&port_id)
        .map(|entry| entry.listeners.snapshots(event_type))
        .unwrap_or_default()
}

pub(crate) fn claim_worker_message_port_event_listener(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    listener_id: crate::context_bootstrap::MessagePortEventListenerId,
) -> Option<crate::context_bootstrap::PreparedMessagePortEventListener> {
    let state = get_worker_state(scope)?;
    let (abort, claimed) = {
        let mut state = state.borrow_mut();
        let abort = state.abort.clone();
        let claimed = state
            .message_port_wrappers
            .get_mut(&port_id)?
            .listeners
            .claim(scope, listener_id);
        (abort, claimed)
    };
    let (prepared, removed_once) = claimed?;
    if removed_once {
        abort
            .borrow_mut()
            .unregister_message_port_listener(port_id, listener_id);
    }
    Some(prepared)
}

pub(crate) fn register_worker_broadcast_channel_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
    channel: v8::Local<'_, v8::Object>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    state
        .borrow_mut()
        .broadcast_channel_wrappers
        .insert(channel_id, v8::Global::new(scope, channel));
}

pub(crate) fn forget_worker_broadcast_channel_wrapper(
    scope: &mut v8::PinScope<'_, '_>,
    channel_id: BroadcastChannelId,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    state
        .borrow_mut()
        .broadcast_channel_wrappers
        .remove(&channel_id);
}

pub(crate) fn worker_broadcast_channel_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channel_id: BroadcastChannelId,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    state
        .broadcast_channel_wrappers
        .get(&channel_id)
        .map(|handle| v8::Local::new(scope, handle))
}

pub(super) fn close_worker_owned_message_ports(state: &Rc<RefCell<WorkerGlobalState>>) {
    let (registry, port_ids): (SharedMessagePortRegistry, Vec<MessagePortId>) = {
        let mut state = state.borrow_mut();
        let shared_worker_connection_ports =
            std::mem::take(&mut state.shared_worker_connection_ports);
        (
            state.message_port_registry.clone(),
            state
                .message_port_wrappers
                .drain()
                .filter_map(|(port_id, _)| {
                    (!shared_worker_connection_ports.contains(&port_id)).then_some(port_id)
                })
                .collect(),
        )
    };
    for port_id in port_ids {
        registry.close_message_port(port_id);
    }
}

pub(super) fn close_worker_owned_broadcast_channels(state: &Rc<RefCell<WorkerGlobalState>>) {
    let (registry, channel_ids): (SharedBroadcastChannelRegistry, Vec<BroadcastChannelId>) = {
        let mut state = state.borrow_mut();
        (
            state.broadcast_channel_registry.clone(),
            state
                .broadcast_channel_wrappers
                .drain()
                .map(|(channel_id, _)| channel_id)
                .collect(),
        )
    };
    for channel_id in channel_ids {
        registry.close_broadcast_channel(channel_id);
    }
}

fn next_fetch_id(state: &mut WorkerGlobalState) -> u32 {
    state.next_fetch_id = state
        .next_fetch_id
        .checked_add(1)
        .expect("worker fetch id space exhausted");
    state.next_fetch_id
}

fn next_xhr_id(state: &mut WorkerGlobalState) -> u32 {
    state.next_xhr_id = state
        .next_xhr_id
        .checked_add(1)
        .expect("worker XHR id space exhausted");
    state.next_xhr_id
}

fn next_websocket_id(state: &mut WorkerGlobalState) -> u64 {
    state.next_websocket_id = state
        .next_websocket_id
        .checked_add(1)
        .expect("worker WebSocket id space exhausted");
    state.next_websocket_id
}

fn worker_url_blocked(patterns: &[String], url: &Url) -> bool {
    let value = url.as_str();
    patterns
        .iter()
        .any(|pattern| moli_fetch::url_pattern_matches(pattern, value))
}

fn merge_worker_request_headers(
    context_headers: &[(String, String)],
    request_headers: &[(String, String)],
) -> Vec<(String, String)> {
    let mut merged = context_headers.to_vec();
    for (name, value) in request_headers {
        let lower = name.to_ascii_lowercase();
        merged.retain(|(existing_name, _)| existing_name.to_ascii_lowercase() != lower);
        merged.push((name.clone(), value.clone()));
    }
    merged
}

fn worker_post_message_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'postMessage' on 'DedicatedWorkerGlobalScope': 1 argument required, but only 0 present.",
        );
        return;
    }
    let val = args.get(0);
    let transfer_arg = (args.length() > 1).then(|| args.get(1));
    let Some(data) = crate::context_bootstrap::structured_serialize_value_for_post_message(
        scope,
        val,
        transfer_arg,
        "DedicatedWorkerGlobalScope",
    ) else {
        return;
    };
    let _ = state
        .borrow()
        .parent_tx
        .send(WorkerToParentMessage::Post(data));
}

fn worker_structured_clone_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = crate::context_bootstrap::structured_clone_value_with_options(
        scope,
        args.get(0),
        args.get(1),
    ) {
        rv.set(value);
    } else {
        rv.set_undefined();
    }
}

fn worker_import_scripts_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    if state.borrow().script_kind == super::thread::WorkerScriptKind::Module {
        throw_type_error(scope, "Module scripts don't support importScripts().");
        return;
    }
    let require_trusted_types_for_script =
        crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
            &state.borrow().content_security_policies,
        );
    let mut prepared = Vec::with_capacity(args.length() as usize);
    for i in 0..args.length() {
        let Some(specifier) = crate::context_bootstrap::trusted_script_url_string_or_throw(
            scope,
            args.get(i),
            crate::content_security_policy::TrustedTypesForScriptRequirements::enforced_only(
                require_trusted_types_for_script,
            ),
            "WorkerGlobalScope importScripts",
            "importScripts",
        ) else {
            return;
        };
        let resolved_url = match resolve_import_script_url(state.clone(), &specifier) {
            Ok(url) => url,
            Err(error) => {
                error.throw(scope);
                return;
            }
        };
        let source = if matches!(resolved_url.scheme(), "data" | "blob") {
            match materialize_worker_import_source(scope, &state, &resolved_url) {
                Ok(import_source) => Some(import_source.source),
                Err(error) => {
                    error.throw(scope);
                    return;
                }
            }
        } else {
            None
        };
        prepared.push(PreparedWorkerImportScript {
            final_url: resolved_url,
            source,
            muted_errors: false,
        });
    }
    for mut script in prepared {
        if script.source.is_none() {
            let import_source =
                match materialize_worker_import_source(scope, &state, &script.final_url) {
                    Ok(result) => result,
                    Err(error) => {
                        error.throw(scope);
                        return;
                    }
                };
            script.final_url = import_source.final_url;
            script.source = Some(import_source.source);
            script.muted_errors = import_source.muted_errors;
        }
        let source = script.source.as_deref().unwrap_or_default();
        if let Err(error) = evaluate_worker_script(
            scope,
            state.clone(),
            &script.final_url,
            source,
            script.muted_errors,
        ) {
            error.throw(scope);
            return;
        }
    }
}

// ─── close ──────────────────────────────────────────────────────────────────

fn worker_close_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(state) = get_worker_state(scope) {
        state.borrow_mut().closed = true;
        close_worker_owned_broadcast_channels(&state);
    }
}

fn install_console<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let console_key = v8str(scope, "console");
    if let Some(original_console) = global.get(scope, console_key.into()) {
        set_private_value(
            scope,
            global,
            WORKER_ORIGINAL_CONSOLE_SLOT,
            original_console,
        );
    }
    let _ = global.delete(scope, console_key.into());

    let console = WorkerConsoleObjectDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to create worker console: {error}"))?;

    WorkerGlobalConsoleDeclaration::new(console)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker console: {error}"))
}

fn install_worker_performance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let time_origin = monotonic_unix_epoch_millis();
    let performance = WorkerPerformanceObjectDeclaration::new(time_origin)
        .bind(scope)
        .map_err(|error| anyhow!("failed to create worker performance: {error}"))?;
    WorkerGlobalPerformanceDeclaration::new(performance)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker performance: {error}"))
}

fn worker_performance_now_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let time_origin = args.data().number_value(scope).unwrap_or(0.0);
    rv.set(
        v8::Number::new(
            scope,
            (monotonic_unix_epoch_millis() - time_origin).max(0.0),
        )
        .into(),
    );
}

fn unix_epoch_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn monotonic_unix_epoch_millis() -> f64 {
    static BASE: OnceLock<(f64, Instant)> = OnceLock::new();
    let (epoch_millis, instant) = BASE.get_or_init(|| (unix_epoch_millis(), Instant::now()));
    epoch_millis + instant.elapsed().as_secs_f64() * 1000.0
}

fn format_console_args(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> String {
    let mut parts = Vec::new();
    for i in 0..args.length() {
        let val = args.get(i);
        if let Some(s) = val.to_detail_string(scope) {
            parts.push(s.to_rust_string_lossy(scope));
        }
    }
    parts.join(" ")
}

fn console_arg_snapshots_json(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Vec<serde_json::Value> {
    let mut snapshots = Vec::with_capacity(args.length().max(0) as usize);
    for index in 0..args.length() {
        snapshots.push(crate::context_bootstrap::console_arg_remote_object_json(
            scope,
            args.get(index),
        ));
    }
    snapshots
}

fn console_log_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let level = args
        .data()
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "log".to_owned());
    let text = format_console_args(scope, &args);
    let message = format!("{level}: {text}");
    let arg_snapshots = console_arg_snapshots_json(scope, &args);
    let stack = crate::context_bootstrap::current_console_stack(scope);
    tracing::trace!("{}", message);
    if let Some(state) = get_worker_state(scope) {
        let parent_tx = state.borrow().parent_tx.clone();
        let _ = parent_tx.send(WorkerToParentMessage::Console(WorkerConsoleMessage {
            message,
            args: arg_snapshots,
            stack,
        }));
    }
}

fn console_noop_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

fn console_profile_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    call_original_worker_console_method(scope, &args, "profile");
    rv.set_undefined();
}

fn console_profile_end_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    call_original_worker_console_method(scope, &args, "profileEnd");
    rv.set_undefined();
}

fn call_original_worker_console_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method_name: &'static str,
) {
    let global = scope.get_current_context().global(scope);
    let Some(original_console) = get_private_value(scope, global, WORKER_ORIGINAL_CONSOLE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(method) = original_console
        .get(scope, v8str(scope, method_name).into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };

    let mut forwarded_args = Vec::with_capacity(args.length().max(0) as usize);
    for index in 0..args.length() {
        forwarded_args.push(args.get(index));
    }
    let _ = method.call(scope, original_console.into(), &forwarded_args);
}

// ─── timers (minimal stubs) ─────────────────────────────────────────────────

fn create_script_origin<'s>(scope: &mut v8::PinScope<'s, '_>, url: &str) -> v8::ScriptOrigin<'s> {
    let name = v8::String::new(scope, url).expect("worker script origin");
    v8::ScriptOrigin::new(
        scope,
        name.into(),
        0,
        0,
        false,
        -1,
        None,
        false,
        false,
        false,
        None,
    )
}

fn set_prop(
    scope: &mut v8::PinScope<'_, '_>,
    obj: v8::Local<'_, v8::Object>,
    name: &str,
    value: v8::Local<'_, v8::Value>,
) -> Result<()> {
    let key =
        v8_string(scope, name).ok_or_else(|| anyhow!("failed to allocate worker key `{name}`"))?;
    obj.define_own_property(scope, key.into(), value, v8::PropertyAttribute::DONT_ENUM)
        .unwrap_or(false)
        .then_some(())
        .ok_or_else(|| anyhow!("failed to set worker global `{name}`"))
}

// ─── timer info (used by the event loop in thread.rs) ───────────────────────

#[derive(Clone, Default)]
pub(super) struct WorkerIsolateTimerQueues {
    pending: Rc<RefCell<Vec<TimerInfo>>>,
    cancelled: Rc<RefCell<Vec<u32>>>,
}

impl WorkerIsolateTimerQueues {
    pub(super) fn push_pending(&self, timer: TimerInfo) {
        self.pending.borrow_mut().push(timer);
    }

    pub(super) fn clear_pending_and_active(&self, timer_id: u32) {
        self.pending
            .borrow_mut()
            .retain(|timer| timer.id != timer_id);
        self.cancel_active(timer_id);
    }

    pub(super) fn cancel_active(&self, timer_id: u32) {
        self.cancelled.borrow_mut().push(timer_id);
    }

    pub(super) fn drain_pending(&self) -> Vec<TimerInfo> {
        self.pending.borrow_mut().drain(..).collect()
    }

    pub(super) fn drain_cancelled(&self) -> Vec<u32> {
        self.cancelled.borrow_mut().drain(..).collect()
    }
}

pub(super) fn worker_isolate_timer_queues(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<WorkerIsolateTimerQueues> {
    scope.get_slot::<WorkerIsolateTimerQueues>().cloned()
}

pub(super) struct TimerInfo {
    pub(super) id: u32,
    pub(super) callback: super::timer_callback::WorkerTimerCallback,
    pub(super) delay_ms: u64,
    pub(super) is_interval: bool,
    pub(super) extra_args: Vec<v8::Global<v8::Value>>,
}
