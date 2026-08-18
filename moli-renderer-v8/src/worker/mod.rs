//! Dedicated Web Worker implementation.
//!
//! Each worker runs on its own OS thread with a dedicated Tokio `LocalRuntime`
//! and V8 isolate, matching Chromium's threading model.  Communication with the
//! parent context uses `tokio::sync::mpsc` channels carrying V8 structured-clone
//! payloads.
//! payload bytes, so `postMessage` semantics are not limited to JSON.
//!
//! The worker's V8 isolate is registered with the custom V8 platform
//! (`v8_platform.rs`) so foreground tasks posted by V8 background threads are
//! routed to the worker's own Tokio runtime — no manual pumping required.

pub(crate) mod abort;
mod data_url;
mod global_scope;
mod handle;
mod inspector_task_runner;
mod module_mime;
mod module_runtime;
mod script_loading;
mod script_mime;
mod thread;
mod timer_callback;

pub(crate) use data_url::decode_data_url_script_source;
pub(crate) use global_scope::{
    NestedWorkerContext, WORKER_STATE_SLOT, WorkerOpfsCompletion, WorkerWebCryptoCompletion,
    cancel_worker_opfs_task, check_worker_websocket_csp, claim_worker_message_port_event_listener,
    close_worker_websocket, dispatch_worker_trusted_types_sink_violation_event,
    ensure_worker_opfs_directory_iterator_registry, ensure_worker_opfs_handle_registry,
    forget_nested_worker_context, forget_worker_broadcast_channel_wrapper,
    forget_worker_message_port_wrapper, get_worker_state,
    register_worker_broadcast_channel_wrapper, register_worker_message_port_event_listener,
    register_worker_message_port_wrapper, register_worker_opfs_iterator_task,
    register_worker_opfs_move_task, register_worker_opfs_task, register_worker_webcrypto_task,
    register_worker_websocket, remove_worker_message_port_event_listener,
    remove_worker_message_port_event_listener_by_id, reserve_nested_worker_context,
    send_worker_websocket_binary, send_worker_websocket_text, service_worker_runtime_identity,
    try_worker_xhr_abort_callback, try_worker_xhr_reschedule_timeout_after_timeout_change,
    try_worker_xhr_send_callback, worker_allows_trusted_type_policy_name,
    worker_allows_trusted_types_eval, worker_broadcast_channel_registry,
    worker_broadcast_channel_storage_key, worker_broadcast_channel_wake_sender,
    worker_broadcast_channel_wrapper, worker_current_script_url, worker_global_is_closed,
    worker_message_port_event_listener_snapshots, worker_message_port_registry,
    worker_message_port_wake_sender, worker_message_port_wrapper,
    worker_notification_permission_state, worker_opfs_directory_iterator_registry,
    worker_opfs_handle_registry, worker_service_worker_control_state, worker_storage_key,
    worker_storage_partition_identity, worker_termination_requested,
    worker_uses_shared_worker_agent_cluster,
};
pub(crate) use handle::WorkerMessage;
pub(crate) use handle::{
    WorkerBootstrapCompletion, WorkerBootstrapFailure, WorkerBootstrapSuccess,
    WorkerConsoleMessage, WorkerErrorPhase, WorkerErrorSource, WorkerFetchHandlerType,
    WorkerParentErrorEventKind, WorkerPendingFetchContinue, WorkerPendingSubresourceFetch,
    WorkerPendingXhrContinue, WorkerRuntimeEvent, WorkerRuntimeInspectorMessageBatch,
    WorkerScriptResource, WorkerScriptResourceKind, WorkerToParentMessage,
    WorkerWebSocketFrameEvent, WorkerWebSocketLifecycleEvent, worker_secure_context_for_script_url,
};
pub(crate) use handle::{WorkerDevToolsHandle, WorkerHandle, WorkerNetworkPolicy};
pub(crate) use module_mime::{
    ensure_worker_css_module_mime, ensure_worker_json_module_mime, ensure_worker_text_module_mime,
    ensure_worker_wasm_module_mime,
};
pub(crate) use module_runtime::worker_wasm_instance_for_namespace;
pub(crate) use script_loading::ensure_worker_script_redirect_chain_same_origin;
pub(crate) use script_mime::{
    ensure_worker_script_mime_acceptable, worker_response_has_webassembly_mime,
};
pub(crate) use thread::{
    WorkerGlobalKind, WorkerScriptKind, WorkerScriptSource, WorkerSpawnOptions,
    dispatch_current_worker_callback_exception, spawn_worker_with_options,
};
