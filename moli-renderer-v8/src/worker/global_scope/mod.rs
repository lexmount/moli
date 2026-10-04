//! WorkerGlobalScope bindings for dedicated, shared and service workers.
//!
//! Initialization, runtime state and API implementations live in the child modules.
//! This module retains the existing worker-facing exports and shared imports.

use crate::web_api_interfaces;
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
use moli_websocket::{
    ConnectOptions as WebSocketConnectOptions, ConnectionHandle as WebSocketConnectionHandle,
    Event as WebSocketEvent, spawn_connection, spawn_failed_connection, websocket_cookie_url,
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
    close_pending_network_body_stream, convert_xhr_send_body_from_args, dispatch_xhr_loadstart,
    dispatch_xhr_upload_abort_if_in_progress, dispatch_xhr_upload_complete,
    enqueue_pending_network_body_chunk, error_pending_network_body_stream_with_reason,
    extract_subresource_auth_challenge,
    fetch_browser_subresource_raw_stream_with_preflight_headers_and_network_metadata,
    fetch_browser_subresource_with_preflight_headers_and_network_metadata,
    filter_cors_exposed_response_headers, filter_headers_for_guard, is_cors_policy_failure_message,
    local_url_response_result, parse_fetch_init, request_input_snapshot,
    request_object_credentials_mode, reset_xhr_response_for_request_error, resolve_context_url,
    set_xhr_state_bool, set_xhr_state_number, throw_synchronous_xhr_failure,
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

mod async_tasks;
mod bootstrap;
mod console;
mod content_security_policy;
mod event_handlers;
mod extendable_events;
mod fetch;
mod import_scripts;
mod messaging;
mod nested_workers;
mod network_state;
mod origin;
mod performance;
mod service_worker_api;
mod service_worker_results;
mod state;
mod timers;
mod xhr;

use self::service_worker_api::*;
pub(super) use self::service_worker_api::{
    build_service_worker_client_object, build_service_worker_client_object_from_snapshot,
    build_service_worker_global_service_worker,
};
pub(crate) use self::service_worker_api::{
    service_worker_runtime_identity, worker_notification_permission_state,
};

use content_security_policy::*;
pub(in crate::worker) use content_security_policy::{
    continue_pending_worker_csp_report, fail_pending_worker_csp_report,
    fulfill_pending_worker_csp_report,
};
pub(crate) use fetch::*;
use import_scripts::*;
use timers::*;
pub(crate) use xhr::*;

pub(super) use async_tasks::{
    PendingWorkerWebCryptoTask, WorkerOpfsOwnerState, drain_worker_opfs_completion,
    drain_worker_webcrypto_completion,
};
pub(crate) use async_tasks::{
    WorkerOpfsCompletion, WorkerWebCryptoCompletion, cancel_worker_opfs_task,
    ensure_worker_opfs_directory_iterator_registry, ensure_worker_opfs_handle_registry,
    register_worker_opfs_iterator_task, register_worker_opfs_move_task, register_worker_opfs_task,
    register_worker_webcrypto_task, worker_opfs_directory_iterator_registry,
    worker_opfs_handle_registry,
};
use bootstrap::{
    constructor_prototype, ensure_worker_interface_constructor, set_worker_to_string_tag,
};
pub(super) use bootstrap::{install_worker_global_scope, prepare_worker_global_scope_templates};
use console::install_console;
pub(super) use content_security_policy::{
    dispatch_worker_csp_violation_event, dispatch_worker_csp_violation_event_for_state,
};
pub(crate) use content_security_policy::{
    dispatch_worker_trusted_types_sink_violation_event, worker_allows_eval_code_generation_by_csp,
    worker_allows_trusted_type_policy_name, worker_allows_trusted_types_eval,
    worker_trusted_types_for_script_requirements,
};
use event_handlers::install_worker_global_event_handler_accessors;
pub(super) use event_handlers::{WORKER_GLOBAL_LISTENERS_SLOT, service_worker_fetch_handler_type};
use extendable_events::install_service_worker_extendable_event_constructors;
pub(super) use extendable_events::{
    PendingServiceWorkerFetchEvent, PendingServiceWorkerLifecycleEvent,
    PendingServiceWorkerMessageEvent, PendingServiceWorkerNavigationPreload,
    PendingServiceWorkerNotificationEvent, PendingServiceWorkerPeriodicSyncEvent,
    PendingServiceWorkerPushEvent, PendingServiceWorkerSyncEvent,
};
pub(super) use import_scripts::{
    WORKER_EXCEPTION_COLUMN_SLOT, WORKER_EXCEPTION_LINE_SLOT, WORKER_EXCEPTION_SOURCE_SLOT,
};
use messaging::{DedicatedWorkerGlobalPostMessageDeclaration, worker_structured_clone_callback};
pub(super) use messaging::{
    WorkerMessagePortWrapperEntry, close_worker_owned_broadcast_channels,
    close_worker_owned_message_ports,
};
pub(crate) use messaging::{
    forget_worker_broadcast_channel_wrapper, forget_worker_message_port_wrapper,
    register_shared_worker_connection_port, register_worker_broadcast_channel_wrapper,
    register_worker_message_port_wrapper, worker_broadcast_channel_registry,
    worker_broadcast_channel_storage_key, worker_broadcast_channel_wake_sender,
    worker_broadcast_channel_wrapper, worker_message_port_registry,
    worker_message_port_wake_sender, worker_message_port_wrapper,
};
pub(super) use nested_workers::dispatch_nested_worker_event;
pub(crate) use nested_workers::{
    NestedWorkerContext, forget_nested_worker_context, reserve_nested_worker_context,
};
pub(super) use network_state::{
    PausedWorkerSubresourceResponse, PendingWorkerCspReport, PendingWorkerFetch,
    PendingWorkerFetchNetworkRecord, PendingWorkerXhr, WorkerFetchCompletion, WorkerFetchEvent,
    WorkerFetchResponse, WorkerFetchStreamingChunk, WorkerFetchStreamingFinished,
    WorkerFetchStreamingStarted, WorkerWebSocketState, WorkerXhrCompletion, WorkerXhrResponse,
};
use network_state::{
    WorkerFetchResponseParts, merge_worker_request_headers, next_fetch_id, next_websocket_id,
    next_xhr_id, worker_url_blocked,
};
pub(crate) use origin::worker_global_origin;
use origin::{WorkerGlobalOriginDeclaration, WorkerGlobalOriginPrototypeDeclaration};
use performance::{install_worker_performance, monotonic_unix_epoch_millis};
pub(super) use service_worker_results::{
    PendingServiceWorkerClientFocus, PendingServiceWorkerClientNavigate,
    PendingServiceWorkerClientQuery, PendingServiceWorkerClientQueryType,
    PendingServiceWorkerClientsOpenWindow, PendingServiceWorkerGetNotifications,
    PendingServiceWorkerPeriodicSyncGetTags, PendingServiceWorkerPeriodicSyncRegistration,
    PendingServiceWorkerPeriodicSyncUnregistration, PendingServiceWorkerPushGetSubscription,
    PendingServiceWorkerPushSubscribe, PendingServiceWorkerPushUnsubscribe,
    PendingServiceWorkerShowNotification, PendingServiceWorkerSyncGetTags,
    PendingServiceWorkerSyncRegistration, WorkerServiceWorkerRequestIdAllocator,
    drain_service_worker_client_focus_result, drain_service_worker_client_navigate_result,
    drain_service_worker_client_query_result, drain_service_worker_clients_open_window_result,
    drain_service_worker_get_notifications_result,
    drain_service_worker_periodic_sync_get_tags_result,
    drain_service_worker_periodic_sync_registration_result,
    drain_service_worker_periodic_sync_unregistration_result,
    drain_service_worker_push_get_subscription_result, drain_service_worker_push_subscribe_result,
    drain_service_worker_push_unsubscribe_result, drain_service_worker_show_notification_result,
    drain_service_worker_sync_get_tags_result, drain_service_worker_sync_registration_result,
};
use state::worker_close_callback;
pub(crate) use state::{
    WORKER_STATE_SLOT, WorkerGlobalState, get_worker_state, worker_current_script_url,
    worker_exception_report_target, worker_global_is_closed, worker_realm_secure_context_available,
    worker_service_worker_control_state, worker_storage_key, worker_storage_partition_identity,
    worker_termination_requested, worker_uses_shared_worker_agent_cluster,
};
pub(super) use timers::{TimerInfo, WorkerIsolateTimerQueues, worker_isolate_timer_queues};
