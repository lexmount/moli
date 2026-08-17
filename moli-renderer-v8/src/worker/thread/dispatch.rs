use std::{
    cell::{Cell, RefCell},
    pin::pin,
    rc::Rc,
};

use tokio::sync::mpsc;

use moli_webapi_declare::WebApiObject;

use crate::callback_invocation::{CallbackInvocationOutcome, CallbackInvoker};
use crate::context_bootstrap::{
    EVENT_DISPATCHING_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, EVENT_STOP_PROPAGATION_SLOT,
    SimpleObjectEventListenerSnapshot, dispatch_message_port_events_for_port_collecting_errors,
    dispatch_service_worker_controller_change, ensure_message_port_wrapper_for_id,
    event_internal_bool_flag, mark_event_trusted, runtime_message_allowed_for_current_target,
    set_event_internal_flag, simple_object_event_listeners_snapshot,
    simple_object_event_remove_listener_value_for_type,
    structured_deserialize_value_for_message_event,
};
use crate::exception_reporting::{
    CallbackExceptionLogLevel, V8ExceptionReport, invoke_callback_with_report,
    log_unhandled_promise_rejection,
};
use crate::network_host::{
    MaterializedResponseBody, MaterializedResponseHead,
    build_navigation_preload_response_object_from_stream_for_request_mode,
    close_pending_network_body_stream, enqueue_pending_network_body_chunk,
    error_pending_network_body_stream_with_reason, materialize_response_object_body,
    materialize_response_object_body_with_chunk_callback,
    materialize_response_object_head_for_service_worker_respond_with,
    materialized_body_bytes_from_value, new_network_body_source_id,
    set_request_destination_for_service_worker_fetch_event,
    set_request_mode_for_service_worker_fetch_event,
    set_request_reload_navigation_for_service_worker_fetch_event,
};
use crate::runtime::{
    MaterializedServiceWorkerFetchResponseHead, ServiceWorkerEventId, ServiceWorkerFetchCompletion,
    ServiceWorkerFetchEvent, ServiceWorkerFetchResponse, ServiceWorkerFetchResult,
    ServiceWorkerFetchStreamChunk, ServiceWorkerFetchStreamStarted,
    ServiceWorkerLifecycleCompletion, ServiceWorkerLifecycleEvent, ServiceWorkerLifecycleEventKind,
    ServiceWorkerMessageCompletion, ServiceWorkerMessageEvent,
    ServiceWorkerNavigationPreloadFailure, ServiceWorkerNavigationPreloadResponseStarted,
    ServiceWorkerNavigationPreloadStreamChunk, ServiceWorkerNavigationPreloadStreamFinished,
    ServiceWorkerNotificationCompletion, ServiceWorkerNotificationEvent,
    ServiceWorkerPeriodicSyncCompletion, ServiceWorkerPeriodicSyncEvent,
    ServiceWorkerPushCompletion, ServiceWorkerPushEvent, ServiceWorkerSyncCompletion,
    ServiceWorkerSyncEvent, service_worker_exposed_client_id,
};
use crate::structured_clone::V8StructuredClonePayload;
use crate::types::{BroadcastChannelId, MessagePortId, NetworkBodySourceId};
use crate::util::{
    get_private_value, serialize_v8_array, set_private_value, throw_type_error, v8str,
};
use crate::webidl::v8_string;
use crate::worker::abort::{
    abort_worker_signal_by_id, worker_abort_error_value, worker_abort_signal_id,
};
use tracing::trace;

use super::super::global_scope::{
    PendingServiceWorkerFetchEvent, PendingServiceWorkerLifecycleEvent,
    PendingServiceWorkerMessageEvent, PendingServiceWorkerNavigationPreload,
    PendingServiceWorkerNotificationEvent, PendingServiceWorkerPeriodicSyncEvent,
    PendingServiceWorkerPushEvent, PendingServiceWorkerSyncEvent, WORKER_EXCEPTION_COLUMN_SLOT,
    WORKER_EXCEPTION_LINE_SLOT, WORKER_EXCEPTION_SOURCE_SLOT, WORKER_GLOBAL_LISTENERS_SLOT,
    get_worker_state, register_shared_worker_connection_port, worker_exception_report_target,
};
use super::super::handle::{
    WorkerErrorPhase, WorkerErrorSource, WorkerMessage, WorkerParentErrorEventKind,
    WorkerToParentMessage,
};
use super::super::timer_callback::WorkerTimerCallbackOutcome;
use super::ActiveTimer;
use super::WorkerExceptionError;

const SERVICE_WORKER_LIFECYCLE_EVENT_ID_SLOT: &str = "__lmServiceWorkerLifecycleEventId";
const SERVICE_WORKER_FETCH_EVENT_ID_SLOT: &str = "__lmServiceWorkerFetchEventId";
const SERVICE_WORKER_MESSAGE_EVENT_ID_SLOT: &str = "__lmServiceWorkerMessageEventId";
const SERVICE_WORKER_NOTIFICATION_EVENT_ID_SLOT: &str = "__lmServiceWorkerNotificationEventId";
const SERVICE_WORKER_PUSH_EVENT_ID_SLOT: &str = "__lmServiceWorkerPushEventId";
const SERVICE_WORKER_PUSH_MESSAGE_DATA_BYTES_SLOT: &str = "__lmServiceWorkerPushMessageDataBytes";
const SERVICE_WORKER_SYNC_EVENT_ID_SLOT: &str = "__lmServiceWorkerSyncEventId";
const SERVICE_WORKER_PERIODIC_SYNC_EVENT_ID_SLOT: &str = "__lmServiceWorkerPeriodicSyncEventId";
// Retain enough reported rejections to populate `rejectionhandled.reason`
// without letting long-lived workers accumulate unbounded promise state.
const MAX_REPORTED_WORKER_PROMISE_REJECTIONS: usize = 1024;

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerMessageEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerErrorEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    message: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    filename: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable, constructor_default = true)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerPromiseRejectionEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    promise: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property, enumerable)]
    reason: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerErrorEventFallbackDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    message: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    filename: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerPromiseRejectionEventFallbackDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    promise: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property, enumerable)]
    reason: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerBasicEventDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable, constructor_default = false)]
    default_prevented: bool,
    #[webapi(method, enumerable, callback = worker_event_prevent_default_callback)]
    prevent_default: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerLifecycleEventDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    default_prevented: bool,
    #[webapi(method, enumerable, callback = worker_event_prevent_default_callback)]
    prevent_default: (),
    #[webapi(method, enumerable, callback = service_worker_wait_until_callback)]
    wait_until: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerFetchEventDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    default_prevented: bool,
    #[webapi(data_property, enumerable)]
    handled: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property, enumerable)]
    preload_response: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property, enumerable)]
    request: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    client_id: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    resulting_client_id: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    is_reload: bool,
    #[webapi(method, enumerable, callback = worker_event_prevent_default_callback)]
    prevent_default: (),
    #[webapi(method, enumerable, callback = service_worker_wait_until_callback)]
    wait_until: (),
    #[webapi(method, enumerable, callback = service_worker_respond_with_callback)]
    respond_with: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerMessageEventDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    origin: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
    #[webapi(data_property, enumerable)]
    default_prevented: bool,
    #[webapi(method, enumerable, callback = worker_event_prevent_default_callback)]
    prevent_default: (),
    #[webapi(method, enumerable, callback = service_worker_wait_until_callback)]
    wait_until: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerExtendableMessageEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    origin: v8::Local<'scope, v8::String>,
    #[webapi(data_property = "lastEventId", enumerable)]
    last_event_id: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerMessageDispatchMethodsDeclaration {
    #[webapi(
        method = "preventDefault",
        callback = worker_event_prevent_default_callback,
        length = 1
    )]
    prevent_default: (),
    #[webapi(
        method = "waitUntil",
        callback = service_worker_wait_until_callback,
        length = 1
    )]
    wait_until: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerNotificationEventDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    notification: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    action: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    default_prevented: bool,
    #[webapi(method, enumerable, callback = worker_event_prevent_default_callback)]
    prevent_default: (),
    #[webapi(method, enumerable, callback = service_worker_wait_until_callback)]
    wait_until: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerPushEventDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    default_prevented: bool,
    #[webapi(method, enumerable, callback = worker_event_prevent_default_callback)]
    prevent_default: (),
    #[webapi(method, enumerable, callback = service_worker_wait_until_callback)]
    wait_until: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "PushMessageData")]
struct ServiceWorkerPushMessageDataDeclaration {
    #[webapi(method = "arrayBuffer", length = 0, callback = push_message_data_array_buffer_callback)]
    array_buffer: (),
    #[webapi(method = "bytes", length = 0, callback = push_message_data_bytes_callback)]
    bytes: (),
    #[webapi(method = "text", length = 0, callback = push_message_data_text_callback)]
    text: (),
    #[webapi(method = "json", length = 0, callback = push_message_data_json_callback)]
    json: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerSyncEventDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    tag: v8::Local<'scope, v8::String>,
    #[webapi(data_property = "lastChance", enumerable)]
    last_chance: bool,
    #[webapi(data_property, enumerable)]
    default_prevented: bool,
    #[webapi(method, enumerable, callback = worker_event_prevent_default_callback)]
    prevent_default: (),
    #[webapi(method, enumerable, callback = service_worker_wait_until_callback)]
    wait_until: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerPeriodicSyncEventDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    tag: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    default_prevented: bool,
    #[webapi(method, enumerable, callback = worker_event_prevent_default_callback)]
    prevent_default: (),
    #[webapi(method, enumerable, callback = service_worker_wait_until_callback)]
    wait_until: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerRespondWithCallbackDataDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    event_id: v8::Local<'scope, v8::BigInt>,
    #[webapi(data_property, enumerable)]
    fulfilled: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerRespondWithLifetimeCallbackDataDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    event_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerRespondWithBodyCallbackDataDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    event_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerRespondWithStreamChunkCallbackDataDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    event_id: v8::Local<'scope, v8::BigInt>,
    #[webapi(data_property, enumerable)]
    body_source_id: v8::Local<'scope, v8::BigInt>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServiceWorkerWaitUntilEventKind {
    Lifecycle,
    Fetch,
    Message,
    Notification,
    Push,
    Sync,
    PeriodicSync,
}

#[derive(Clone)]
struct PendingWorkerPromiseRejection {
    promise: v8::Global<v8::Promise>,
    reason: Option<v8::Global<v8::Value>>,
}

struct WorkerPromiseRejectDispatchSlot {
    parent_tx: mpsc::UnboundedSender<WorkerToParentMessage>,
    worker_wake_tx: mpsc::UnboundedSender<WorkerMessage>,
    script_url: String,
    pending_unhandled_rejections: Rc<RefCell<Vec<PendingWorkerPromiseRejection>>>,
    reported_unhandled_rejections: Rc<RefCell<Vec<PendingWorkerPromiseRejection>>>,
    unhandled_rejection_task_queued: Rc<Cell<bool>>,
}

pub(super) fn install_worker_promise_rejection_dispatch(
    isolate: &mut v8::OwnedIsolate,
    parent_tx: mpsc::UnboundedSender<WorkerToParentMessage>,
    worker_wake_tx: mpsc::UnboundedSender<WorkerMessage>,
    script_url: String,
) {
    isolate.set_slot(WorkerPromiseRejectDispatchSlot {
        parent_tx,
        worker_wake_tx,
        script_url,
        pending_unhandled_rejections: Rc::new(RefCell::new(Vec::new())),
        reported_unhandled_rejections: Rc::new(RefCell::new(Vec::new())),
        unhandled_rejection_task_queued: Rc::new(Cell::new(false)),
    });
    isolate.set_promise_reject_callback(worker_promise_reject_callback);
}

fn worker_promise_reject_dispatch_state(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<(
    mpsc::UnboundedSender<WorkerToParentMessage>,
    String,
    Rc<RefCell<Vec<PendingWorkerPromiseRejection>>>,
    Rc<RefCell<Vec<PendingWorkerPromiseRejection>>>,
    Rc<Cell<bool>>,
)> {
    scope
        .get_slot::<WorkerPromiseRejectDispatchSlot>()
        .map(|slot| {
            (
                slot.parent_tx.clone(),
                slot.script_url.clone(),
                slot.pending_unhandled_rejections.clone(),
                slot.reported_unhandled_rejections.clone(),
                slot.unhandled_rejection_task_queued.clone(),
            )
        })
}

fn worker_promise_reject_task_state(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<(
    mpsc::UnboundedSender<WorkerMessage>,
    Rc<RefCell<Vec<PendingWorkerPromiseRejection>>>,
    Rc<Cell<bool>>,
)> {
    scope
        .get_slot::<WorkerPromiseRejectDispatchSlot>()
        .map(|slot| {
            (
                slot.worker_wake_tx.clone(),
                slot.pending_unhandled_rejections.clone(),
                slot.unhandled_rejection_task_queued.clone(),
            )
        })
}

fn pending_worker_promise_rejection_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rejection: &PendingWorkerPromiseRejection,
    promise: v8::Local<'s, v8::Promise>,
) -> bool {
    v8::Local::new(scope, &rejection.promise).strict_equals(promise.into())
}

pub(super) fn perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(
    scope: &mut v8::PinScope<'_, '_>,
) {
    scope.perform_microtask_checkpoint();
    queue_pending_worker_promise_rejection_task(scope);
    crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
}

fn queue_pending_worker_promise_rejection_task(scope: &mut v8::PinScope<'_, '_>) {
    let Some((worker_wake_tx, pending_unhandled_rejections, task_queued)) =
        worker_promise_reject_task_state(scope)
    else {
        return;
    };
    if pending_unhandled_rejections.borrow().is_empty() || task_queued.get() {
        return;
    }
    task_queued.set(true);
    let _ = worker_wake_tx.send(WorkerMessage::DispatchPendingPromiseRejections);
}

pub(super) fn dispatch_queued_worker_promise_rejections(scope: &mut v8::PinScope<'_, '_>) {
    if let Some((_, _, task_queued)) = worker_promise_reject_task_state(scope) {
        task_queued.set(false);
    }
    flush_pending_worker_promise_rejections(scope);
}

fn flush_pending_worker_promise_rejections(scope: &mut v8::PinScope<'_, '_>) {
    let Some((
        parent_tx,
        script_url,
        pending_unhandled_rejections,
        reported_unhandled_rejections,
        _task_queued,
    )) = worker_promise_reject_dispatch_state(scope)
    else {
        return;
    };
    let pending = std::mem::take(&mut *pending_unhandled_rejections.borrow_mut());

    for rejection in pending {
        let promise = v8::Local::new(scope, &rejection.promise);
        let reason = rejection
            .reason
            .as_ref()
            .map(|reason| v8::Local::new(scope, reason));
        remember_reported_worker_promise_rejection(
            &reported_unhandled_rejections,
            rejection.clone(),
        );
        let allows_default = dispatch_worker_promise_rejection_event(
            scope,
            "unhandledrejection",
            promise,
            reason,
            &parent_tx,
            &script_url,
        );
        if allows_default {
            log_unhandled_promise_rejection(scope, reason);
        }
    }
}

fn remember_reported_worker_promise_rejection(
    reported_unhandled_rejections: &Rc<RefCell<Vec<PendingWorkerPromiseRejection>>>,
    rejection: PendingWorkerPromiseRejection,
) {
    let mut reported = reported_unhandled_rejections.borrow_mut();
    if reported.len() >= MAX_REPORTED_WORKER_PROMISE_REJECTIONS {
        reported.remove(0);
    }
    reported.push(rejection);
}

unsafe extern "C" fn worker_promise_reject_callback(message: v8::PromiseRejectMessage<'_>) {
    let scope = pin!(unsafe { v8::CallbackScope::new(&message) });
    let scope = &mut scope.init();
    let context = scope.get_current_context();
    let scope = &mut v8::ContextScope::new(scope, context);

    let Some((
        parent_tx,
        script_url,
        pending_unhandled_rejections,
        reported_unhandled_rejections,
        _task_queued,
    )) = worker_promise_reject_dispatch_state(scope)
    else {
        return;
    };

    match message.get_event() {
        v8::PromiseRejectEvent::PromiseRejectWithNoHandler => {
            let promise = message.get_promise();
            let mut pending = pending_unhandled_rejections.borrow_mut();
            if pending.iter().any(|rejection| {
                pending_worker_promise_rejection_matches(scope, rejection, promise)
            }) {
                return;
            }
            pending.push(PendingWorkerPromiseRejection {
                promise: v8::Global::new(scope, promise),
                reason: message
                    .get_value()
                    .map(|reason| v8::Global::new(scope, reason)),
            });
        }
        v8::PromiseRejectEvent::PromiseHandlerAddedAfterReject => {
            let promise = message.get_promise();
            let mut pending = pending_unhandled_rejections.borrow_mut();
            if let Some(index) = pending.iter().position(|rejection| {
                pending_worker_promise_rejection_matches(scope, rejection, promise)
            }) {
                pending.swap_remove(index);
                return;
            }
            drop(pending);

            let reported = {
                let mut reported = reported_unhandled_rejections.borrow_mut();
                reported
                    .iter()
                    .position(|rejection| {
                        pending_worker_promise_rejection_matches(scope, rejection, promise)
                    })
                    .map(|index| reported.swap_remove(index))
            };
            let reason = reported
                .as_ref()
                .and_then(|rejection| {
                    rejection
                        .reason
                        .as_ref()
                        .map(|reason| v8::Local::new(scope, reason))
                })
                .or_else(|| message.get_value());
            let _ = dispatch_worker_promise_rejection_event(
                scope,
                "rejectionhandled",
                promise,
                reason,
                &parent_tx,
                &script_url,
            );
        }
        _ => {}
    }
}

pub(super) fn event_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
    key: &str,
) -> bool {
    let key = v8::String::new(scope, key).unwrap();
    event
        .get(scope, key.into())
        .is_some_and(|value| value.is_true())
}

fn event_stop_immediate_propagation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT)
}

fn set_event_stop_immediate_propagation(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) {
    set_event_internal_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT, true);
    set_event_internal_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, true);
}

fn set_event_dispatch_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    mark_event_trusted(scope, event);
    let _ = event.set(scope, v8str(scope, "target").into(), target.into());
    let _ = event.set(scope, v8str(scope, "srcElement").into(), target.into());
    let _ = event.set(scope, v8str(scope, "currentTarget").into(), target.into());
    let _ = event.set(
        scope,
        v8str(scope, "eventPhase").into(),
        v8::Integer::new_from_unsigned(scope, 2).into(),
    );
    set_private_value(
        scope,
        event,
        EVENT_DISPATCHING_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}

fn clear_event_dispatch_fields(scope: &mut v8::PinScope<'_, '_>, event: v8::Local<'_, v8::Object>) {
    let _ = event.set(
        scope,
        v8str(scope, "currentTarget").into(),
        v8::null(scope).into(),
    );
    let _ = event.set(
        scope,
        v8str(scope, "eventPhase").into(),
        v8::Integer::new_from_unsigned(scope, 0).into(),
    );
    set_private_value(
        scope,
        event,
        EVENT_DISPATCHING_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
}

fn worker_event_prevent_default_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    let _ = event.set(
        scope,
        v8str(scope, "defaultPrevented").into(),
        v8::Boolean::new(scope, true).into(),
    );
}

fn new_worker_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
) -> v8::Local<'s, v8::Object> {
    WorkerBasicEventDeclaration::new(v8::String::new(scope, event_type).unwrap())
        .bind(scope)
        .expect("worker basic Event declaration should bind")
}

fn new_worker_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    source: Option<v8::Local<'s, v8::Value>>,
    ports: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(message_ctor) = global
        .get(scope, v8str(scope, "MessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let init = WorkerMessageEventInitDeclaration::new(
            data,
            source.unwrap_or_else(|| v8::null(scope).into()),
            ports,
        )
        .bind(scope)
        .expect("worker MessageEvent init declaration should bind");
        if let Some(event_type) = v8::String::new(scope, event_type)
            && let Some(event) = message_ctor.new_instance(scope, &[event_type.into(), init.into()])
        {
            return event;
        }
    }

    let event = new_worker_event_object(scope, event_type);
    let _ = WorkerMessageEventInitDeclaration::new(
        data,
        source.unwrap_or_else(|| v8::null(scope).into()),
        ports,
    )
    .initialize(scope, event);
    event
}

fn new_worker_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    report: &V8ExceptionReport,
    default_url: &str,
    exception: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    let filename = report.source.as_deref().unwrap_or(default_url);
    let line = report.line.unwrap_or(0);
    let col = report.column.unwrap_or(0);
    if let Some(error_ctor) = global
        .get(scope, v8str(scope, "ErrorEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let init = WorkerErrorEventInitDeclaration::new(
            v8::String::new(scope, &report.summary).unwrap(),
            v8::String::new(scope, filename).unwrap(),
            line as u32,
            col as u32,
            exception.unwrap_or_else(|| v8::undefined(scope).into()),
        )
        .bind(scope)
        .expect("worker ErrorEvent init declaration should bind");
        if let Some(event) = error_ctor.new_instance(
            scope,
            &[v8::String::new(scope, "error").unwrap().into(), init.into()],
        ) {
            return event;
        }
    }

    let event = new_worker_event_object(scope, "error");
    let _ = WorkerErrorEventFallbackDeclaration::new(
        v8::String::new(scope, &report.summary).unwrap(),
        v8::String::new(scope, filename).unwrap(),
        line as u32,
        col as u32,
        exception.unwrap_or_else(|| v8::undefined(scope).into()),
    )
    .initialize(scope, event);
    event
}

fn new_worker_promise_rejection_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    promise: v8::Local<'s, v8::Promise>,
    reason: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(event_ctor) = global
        .get(scope, v8str(scope, "PromiseRejectionEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let init = WorkerPromiseRejectionEventInitDeclaration::new(
            event_type == "unhandledrejection",
            promise,
            reason.unwrap_or_else(|| v8::null(scope).into()),
        )
        .bind(scope)
        .expect("worker PromiseRejectionEvent init declaration should bind");
        if let Some(type_value) = v8::String::new(scope, event_type)
            && let Some(event) = event_ctor.new_instance(scope, &[type_value.into(), init.into()])
        {
            return event;
        }
    }

    let event = new_worker_event_object(scope, event_type);
    let _ = WorkerPromiseRejectionEventFallbackDeclaration::new(
        promise,
        reason.unwrap_or_else(|| v8::null(scope).into()),
    )
    .initialize(scope, event);
    event
}

fn dispatch_worker_promise_rejection_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    promise: v8::Local<'s, v8::Promise>,
    reason: Option<v8::Local<'s, v8::Value>>,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) -> bool {
    let global = scope.get_current_context().global(scope);
    let event = new_worker_promise_rejection_event(scope, event_type, promise, reason);
    set_event_dispatch_fields(scope, global, event);

    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        event_type,
    );
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event,
            "WorkerGlobalScope promise rejection listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                event_type,
                listener.original,
                listener.capture,
            );
        }
    }

    let allows_default = !event_bool_property(scope, event, "defaultPrevented");
    clear_event_dispatch_fields(scope, event);
    allows_default
}

pub(super) fn report_exception_to_parent(
    report: &V8ExceptionReport,
    script_url: &str,
    event_kind: WorkerParentErrorEventKind,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
) {
    report_exception_to_parent_with_phase(
        report,
        script_url,
        event_kind,
        WorkerErrorPhase::Runtime,
        parent_tx,
    );
}

pub(super) fn report_exception_to_parent_with_phase(
    report: &V8ExceptionReport,
    script_url: &str,
    event_kind: WorkerParentErrorEventKind,
    phase: WorkerErrorPhase,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
) {
    report_exception_to_parent_with_phase_and_source(
        report,
        script_url,
        event_kind,
        phase,
        WorkerErrorSource::Runtime,
        parent_tx,
    );
}

pub(super) fn report_exception_to_parent_with_phase_and_source(
    report: &V8ExceptionReport,
    script_url: &str,
    event_kind: WorkerParentErrorEventKind,
    phase: WorkerErrorPhase,
    source: WorkerErrorSource,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
) {
    let _ = parent_tx.send(WorkerToParentMessage::Error {
        message: report.summary.clone(),
        filename: report
            .source
            .clone()
            .unwrap_or_else(|| script_url.to_owned()),
        lineno: report.line.unwrap_or(0) as u32,
        colno: report.column.unwrap_or(0) as u32,
        event_kind,
        phase,
        source,
    });
}

fn worker_exception_source_override<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
) -> Option<String> {
    let object = v8::Local::<v8::Object>::try_from(exception).ok()?;
    get_private_value(scope, object, WORKER_EXCEPTION_SOURCE_SLOT)?
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
}

fn worker_exception_number_override<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
    slot: &str,
) -> Option<usize> {
    let object = v8::Local::<v8::Object>::try_from(exception).ok()?;
    let value = get_private_value(scope, object, slot)?;
    let number = value.number_value(scope)?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    Some(number as usize)
}

fn apply_worker_exception_location_overrides<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    report: &mut V8ExceptionReport,
    exception: Option<v8::Local<'s, v8::Value>>,
) {
    let Some(exception) = exception else {
        return;
    };
    if let Some(source) = worker_exception_source_override(scope, exception) {
        report.source = Some(source);
    }
    if let Some(line) =
        worker_exception_number_override(scope, exception, WORKER_EXCEPTION_LINE_SLOT)
    {
        report.line = Some(line);
    }
    if let Some(column) =
        worker_exception_number_override(scope, exception, WORKER_EXCEPTION_COLUMN_SLOT)
    {
        report.column = Some(column);
    }
}

fn invoke_worker_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    listener: &SimpleObjectEventListenerSnapshot<'s>,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    callback_name: &str,
) -> Result<(), WorkerExceptionError> {
    let arguments = [event.into()];
    let invocation = listener.invocation(target.into(), &arguments, Some(event));
    match CallbackInvoker::invoke(
        scope,
        "event listener",
        "worker event listener threw",
        CallbackExceptionLogLevel::Debug,
        callback_name,
        invocation,
    ) {
        CallbackInvocationOutcome::Returned(_) | CallbackInvocationOutcome::Retired => Ok(()),
        CallbackInvocationOutcome::Threw(report) => {
            let exception = report
                .exception
                .as_ref()
                .map(|exception| v8::Global::new(scope, v8::Local::new(scope, exception)));
            Err(Box::new((*report, exception)))
        }
    }
}

pub(super) fn dispatch_worker_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    report: &V8ExceptionReport,
    exception: Option<v8::Local<'s, v8::Value>>,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) -> bool {
    let event = new_worker_error_event(scope, report, script_url, exception);
    set_event_dispatch_fields(scope, global, event);

    let message = v8::String::new(scope, &report.summary).unwrap();
    let filename = v8::String::new(scope, report.source.as_deref().unwrap_or(script_url)).unwrap();
    let lineno = v8::Integer::new_from_unsigned(scope, report.line.unwrap_or(0) as u32);
    let colno = v8::Integer::new_from_unsigned(scope, report.column.unwrap_or(0) as u32);
    let error_value = exception.unwrap_or_else(|| v8::undefined(scope).into());

    if let Some(handler) = global
        .get(scope, v8str(scope, "onerror").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        match invoke_callback_with_report(
            scope,
            "callback",
            "worker global onerror threw",
            crate::exception_reporting::CallbackExceptionLogLevel::Error,
            "WorkerGlobalScope.onerror",
            handler,
            global.into(),
            &[
                message.into(),
                filename.into(),
                lineno.into(),
                colno.into(),
                error_value,
            ],
        ) {
            Ok(returned) => {
                if v8::Local::new(scope, &returned).is_true() {
                    let _ = event.set(
                        scope,
                        v8str(scope, "defaultPrevented").into(),
                        v8::Boolean::new(scope, true).into(),
                    );
                }
            }
            Err(nested_report) => {
                report_exception_to_parent(
                    &nested_report,
                    script_url,
                    WorkerParentErrorEventKind::ErrorEvent,
                    parent_tx,
                );
            }
        }
    }

    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        "error",
    );
    for listener in &listeners {
        if let Err(nested_report) = invoke_worker_listener(
            scope,
            listener,
            global,
            event,
            "WorkerGlobalScope error listener",
        ) {
            let (nested_report, nested_exception) = *nested_report;
            let _ = nested_exception;
            report_exception_to_parent(
                &nested_report,
                script_url,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                "error",
                listener.original,
                listener.capture,
            );
        }
    }

    let handled = event_bool_property(scope, event, "defaultPrevented");
    clear_event_dispatch_fields(scope, event);
    handled
}

pub(super) fn dispatch_service_worker_lifecycle_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerLifecycleEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let global = ctx.global(scope);
    dispatch_service_worker_lifecycle_event_in_context(
        scope, global, state, event, parent_tx, script_url,
    );
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_service_worker_fetch_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerFetchEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let global = ctx.global(scope);
    dispatch_service_worker_fetch_event_in_context(
        scope, global, state, event, parent_tx, script_url,
    );
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn cancel_service_worker_fetch_stream(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
    body_source_id: NetworkBodySourceId,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    cancel_service_worker_fetch_stream_in_context(scope, state, event_id, body_source_id);
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn abort_service_worker_fetch_request_signal(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
    reason: Option<V8StructuredClonePayload>,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    abort_service_worker_fetch_request_signal_in_context(scope, state, event_id, reason);
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_service_worker_message_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerMessageEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let global = ctx.global(scope);
    dispatch_service_worker_message_event_in_context(
        scope, global, state, event, parent_tx, script_url,
    );
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_service_worker_controller_change_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    dispatch_service_worker_controller_change(scope, crate::native_bridge::OwnerDispatchScope::Top);
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_service_worker_notification_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerNotificationEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let global = ctx.global(scope);
    dispatch_service_worker_notification_event_in_context(
        scope, global, state, event, parent_tx, script_url,
    );
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_service_worker_push_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerPushEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let global = ctx.global(scope);
    dispatch_service_worker_push_event_in_context(
        scope, global, state, event, parent_tx, script_url,
    );
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_service_worker_sync_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerSyncEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let global = ctx.global(scope);
    dispatch_service_worker_sync_event_in_context(
        scope, global, state, event, parent_tx, script_url,
    );
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_service_worker_periodic_sync_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerPeriodicSyncEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let global = ctx.global(scope);
    dispatch_service_worker_periodic_sync_event_in_context(
        scope, global, state, event, parent_tx, script_url,
    );
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

fn dispatch_service_worker_lifecycle_event_in_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerLifecycleEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let event_object = new_service_worker_lifecycle_event(scope, event.kind);
    set_service_worker_event_id(scope, event_object, event.event_id);
    set_event_dispatch_fields(scope, global, event_object);
    {
        let mut state = state.borrow_mut();
        state.pending_service_worker_lifecycle_events.insert(
            event.event_id,
            PendingServiceWorkerLifecycleEvent {
                completion: ServiceWorkerLifecycleCompletion {
                    event_id: event.event_id,
                    version_id: event.version_id,
                    run: event.run.clone(),
                    kind: event.kind,
                    result: Ok(()),
                },
                pending_wait_until_count: 0,
                dispatch_finished: false,
            },
        );
    }

    let event_type = event.kind.as_str();
    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        event_type,
    );
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event_object,
            "ServiceWorkerGlobalScope lifecycle listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                event_type,
                listener.original,
                listener.capture,
            );
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    clear_event_dispatch_fields(scope, event_object);
    mark_service_worker_lifecycle_dispatch_finished(state, event.event_id);
    maybe_send_service_worker_lifecycle_completion(state, event.event_id);
}

fn dispatch_service_worker_fetch_event_in_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerFetchEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let Some((request, request_signal_id)) =
        new_service_worker_fetch_request(scope, global, &event)
    else {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerFetchCompleted(
            ServiceWorkerFetchCompletion {
                event_id: event.event_id,
                version_id: event.version_id,
                run: event.run.clone(),
                result: ServiceWorkerFetchResult::Failure(
                    "failed to construct service worker FetchEvent request".to_owned(),
                ),
            },
        ));
        return;
    };
    let Some(handled_resolver) = v8::PromiseResolver::new(scope) else {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerFetchCompleted(
            ServiceWorkerFetchCompletion {
                event_id: event.event_id,
                version_id: event.version_id,
                run: event.run.clone(),
                result: ServiceWorkerFetchResult::Failure(
                    "failed to construct service worker FetchEvent.handled promise".to_owned(),
                ),
            },
        ));
        return;
    };
    let handled = handled_resolver.get_promise(scope);
    let Some((preload_response, preload_response_resolver)) =
        new_service_worker_preload_response_promise(scope, event.navigation_preload_sent)
    else {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerFetchCompleted(
            ServiceWorkerFetchCompletion {
                event_id: event.event_id,
                version_id: event.version_id,
                run: event.run.clone(),
                result: ServiceWorkerFetchResult::Failure(
                    "failed to construct service worker FetchEvent.preloadResponse promise"
                        .to_owned(),
                ),
            },
        ));
        return;
    };
    let event_object =
        new_service_worker_fetch_event(scope, &event, request, handled, preload_response);
    set_service_worker_fetch_event_id(scope, event_object, event.event_id);
    set_event_dispatch_fields(scope, global, event_object);
    {
        let mut state = state.borrow_mut();
        let mut pending = PendingServiceWorkerFetchEvent::fallback(
            ServiceWorkerFetchCompletion {
                event_id: event.event_id,
                version_id: event.version_id,
                run: event.run.clone(),
                result: ServiceWorkerFetchResult::Fallback,
            },
            event.request.request_mode,
            event.request.destination,
        );
        pending.handled_resolver = Some(v8::Global::new(scope, handled_resolver));
        if event.navigation_preload_sent {
            state.pending_service_worker_navigation_preloads.insert(
                event.event_id,
                PendingServiceWorkerNavigationPreload {
                    version_id: event.version_id,
                    run: event.run.clone(),
                    _promise: v8::Global::new(scope, preload_response),
                    resolver: Some(v8::Global::new(scope, preload_response_resolver)),
                    body_source_id: None,
                },
            );
        }
        pending.request_signal_id = request_signal_id;
        state
            .pending_service_worker_fetch_events
            .insert(event.event_id, pending);
    }

    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        "fetch",
    );
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event_object,
            "ServiceWorkerGlobalScope fetch listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                "fetch",
                listener.original,
                listener.capture,
            );
        }
        if event_stop_immediate_propagation(scope, event_object) {
            break;
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    let default_prevented_without_response =
        event_bool_property(scope, event_object, "defaultPrevented");
    if default_prevented_without_response {
        mark_service_worker_fetch_default_prevented(state, event.event_id);
    }
    clear_event_dispatch_fields(scope, event_object);
    mark_service_worker_fetch_dispatch_finished(state, event.event_id);
    settle_service_worker_fetch_handled_after_dispatch(scope, state, event.event_id);
    maybe_send_service_worker_fetch_completion(state, event.event_id, scope);
}

fn dispatch_service_worker_message_event_in_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerMessageEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let (event_type, event_object) = new_service_worker_message_event(scope, &event)
        .unwrap_or_else(|| {
            (
                "messageerror",
                new_service_worker_messageerror_event(scope, &event),
            )
        });
    set_service_worker_message_event_id(scope, event_object, event.event_id);
    set_event_dispatch_fields(scope, global, event_object);
    {
        let mut state = state.borrow_mut();
        state.pending_service_worker_message_events.insert(
            event.event_id,
            PendingServiceWorkerMessageEvent {
                completion: ServiceWorkerMessageCompletion {
                    event_id: event.event_id,
                    version_id: event.version_id,
                    run: event.run.clone(),
                    result: Ok(()),
                },
                pending_wait_until_count: 0,
                dispatch_finished: false,
                window_interaction_allowed: event.window_interaction_allowed,
            },
        );
        if event.window_interaction_allowed {
            state.service_worker_window_interaction_allowed_count = state
                .service_worker_window_interaction_allowed_count
                .saturating_add(1);
        }
    }

    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        event_type,
    );
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event_object,
            "ServiceWorkerGlobalScope message listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                event_type,
                listener.original,
                listener.capture,
            );
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    clear_event_dispatch_fields(scope, event_object);
    mark_service_worker_message_dispatch_finished(state, event.event_id);
    maybe_send_service_worker_message_completion(state, event.event_id);
}

fn dispatch_service_worker_notification_event_in_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerNotificationEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let Some(event_object) = new_service_worker_notification_event(scope, global, &event) else {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerNotificationCompleted(
            ServiceWorkerNotificationCompletion {
                event_id: event.event_id,
                version_id: event.version_id,
                run: event.run.clone(),
                result: Err("failed to construct service worker notification event".to_owned()),
            },
        ));
        return;
    };
    set_service_worker_notification_event_id(scope, event_object, event.event_id);
    set_event_dispatch_fields(scope, global, event_object);
    {
        let mut state = state.borrow_mut();
        let window_interaction_allowed = event.kind.grants_window_interaction();
        state.pending_service_worker_notification_events.insert(
            event.event_id,
            PendingServiceWorkerNotificationEvent {
                completion: ServiceWorkerNotificationCompletion {
                    event_id: event.event_id,
                    version_id: event.version_id,
                    run: event.run.clone(),
                    result: Ok(()),
                },
                pending_wait_until_count: 0,
                dispatch_finished: false,
                window_interaction_allowed,
            },
        );
        if window_interaction_allowed {
            state.service_worker_window_interaction_allowed_count = state
                .service_worker_window_interaction_allowed_count
                .saturating_add(1);
        }
    }

    let event_type = event.kind.as_str();
    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        event_type,
    );
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event_object,
            "ServiceWorkerGlobalScope notification listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                event_type,
                listener.original,
                listener.capture,
            );
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    clear_event_dispatch_fields(scope, event_object);
    mark_service_worker_notification_dispatch_finished(state, event.event_id);
    maybe_send_service_worker_notification_completion(state, event.event_id);
}

fn dispatch_service_worker_push_event_in_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerPushEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let Some(event_object) = new_service_worker_push_event(scope, &event) else {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPushCompleted(
            ServiceWorkerPushCompletion {
                event_id: event.event_id,
                version_id: event.version_id,
                run: event.run.clone(),
                result: Err("failed to construct service worker push event".to_owned()),
            },
        ));
        return;
    };
    set_service_worker_push_event_id(scope, event_object, event.event_id);
    set_event_dispatch_fields(scope, global, event_object);
    {
        let mut state = state.borrow_mut();
        state.pending_service_worker_push_events.insert(
            event.event_id,
            PendingServiceWorkerPushEvent {
                completion: ServiceWorkerPushCompletion {
                    event_id: event.event_id,
                    version_id: event.version_id,
                    run: event.run.clone(),
                    result: Ok(()),
                },
                pending_wait_until_count: 0,
                dispatch_finished: false,
            },
        );
    }

    let listeners =
        simple_object_event_listeners_snapshot(scope, global, WORKER_GLOBAL_LISTENERS_SLOT, "push");
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event_object,
            "ServiceWorkerGlobalScope push listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                "push",
                listener.original,
                listener.capture,
            );
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    clear_event_dispatch_fields(scope, event_object);
    mark_service_worker_push_dispatch_finished(state, event.event_id);
    maybe_send_service_worker_push_completion(state, event.event_id);
}

fn dispatch_service_worker_sync_event_in_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerSyncEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let Some(event_object) = new_service_worker_sync_event(scope, &event) else {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerSyncCompleted(
            ServiceWorkerSyncCompletion {
                event_id: event.event_id,
                registration_id: event.registration_id,
                version_id: event.version_id,
                run: event.run.clone(),
                tag: event.tag,
                result: Err("failed to construct service worker sync event".to_owned()),
            },
        ));
        return;
    };
    set_service_worker_sync_event_id(scope, event_object, event.event_id);
    set_event_dispatch_fields(scope, global, event_object);
    {
        let mut state = state.borrow_mut();
        state.pending_service_worker_sync_events.insert(
            event.event_id,
            PendingServiceWorkerSyncEvent {
                completion: ServiceWorkerSyncCompletion {
                    event_id: event.event_id,
                    registration_id: event.registration_id,
                    version_id: event.version_id,
                    run: event.run.clone(),
                    tag: event.tag.clone(),
                    result: Ok(()),
                },
                pending_wait_until_count: 0,
                dispatch_finished: false,
            },
        );
    }

    let listeners =
        simple_object_event_listeners_snapshot(scope, global, WORKER_GLOBAL_LISTENERS_SLOT, "sync");
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event_object,
            "ServiceWorkerGlobalScope sync listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                "sync",
                listener.original,
                listener.capture,
            );
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    clear_event_dispatch_fields(scope, event_object);
    mark_service_worker_sync_dispatch_finished(state, event.event_id);
    maybe_send_service_worker_sync_completion(state, event.event_id);
}

fn dispatch_service_worker_periodic_sync_event_in_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event: ServiceWorkerPeriodicSyncEvent,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let Some(event_object) = new_service_worker_periodic_sync_event(scope, &event) else {
        let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(
            ServiceWorkerPeriodicSyncCompletion {
                event_id: event.event_id,
                registration_id: event.registration_id,
                version_id: event.version_id,
                run: event.run.clone(),
                tag: event.tag,
                result: Err("failed to construct service worker periodic sync event".to_owned()),
            },
        ));
        return;
    };
    set_service_worker_periodic_sync_event_id(scope, event_object, event.event_id);
    set_event_dispatch_fields(scope, global, event_object);
    {
        let mut state = state.borrow_mut();
        state.pending_service_worker_periodic_sync_events.insert(
            event.event_id,
            PendingServiceWorkerPeriodicSyncEvent {
                completion: ServiceWorkerPeriodicSyncCompletion {
                    event_id: event.event_id,
                    registration_id: event.registration_id,
                    version_id: event.version_id,
                    run: event.run.clone(),
                    tag: event.tag.clone(),
                    result: Ok(()),
                },
                pending_wait_until_count: 0,
                dispatch_finished: false,
            },
        );
    }

    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        "periodicsync",
    );
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event_object,
            "ServiceWorkerGlobalScope periodicsync listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                "periodicsync",
                listener.original,
                listener.capture,
            );
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    clear_event_dispatch_fields(scope, event_object);
    mark_service_worker_periodic_sync_dispatch_finished(state, event.event_id);
    maybe_send_service_worker_periodic_sync_completion(state, event.event_id);
}

fn new_service_worker_lifecycle_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: ServiceWorkerLifecycleEventKind,
) -> v8::Local<'s, v8::Object> {
    ServiceWorkerLifecycleEventDeclaration {
        r#type: v8::String::new(scope, kind.as_str()).unwrap(),
        default_prevented: false,
        prevent_default: (),
        wait_until: (),
    }
    .bind(scope)
    .expect("service worker lifecycle event declaration should bind")
}

fn new_service_worker_fetch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: &ServiceWorkerFetchEvent,
    request: v8::Local<'s, v8::Object>,
    handled: v8::Local<'s, v8::Promise>,
    preload_response: v8::Local<'s, v8::Promise>,
) -> v8::Local<'s, v8::Object> {
    let client_id = v8::String::new(
        scope,
        &service_worker_exposed_client_id(event.request.client_id),
    )
    .unwrap_or_else(|| v8::String::empty(scope));
    let resulting_client_id = event
        .request
        .resulting_client_id
        .map(service_worker_exposed_client_id)
        .and_then(|client_id| v8::String::new(scope, &client_id))
        .unwrap_or_else(|| v8::String::empty(scope));
    ServiceWorkerFetchEventDeclaration {
        r#type: v8::String::new(scope, "fetch").unwrap(),
        default_prevented: false,
        handled,
        preload_response,
        request,
        client_id,
        resulting_client_id,
        is_reload: event.request.is_reload,
        prevent_default: (),
        wait_until: (),
        respond_with: (),
    }
    .bind(scope)
    .expect("service worker fetch event declaration should bind")
}

fn new_service_worker_preload_response_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation_preload_sent: bool,
) -> Option<(
    v8::Local<'s, v8::Promise>,
    v8::Local<'s, v8::PromiseResolver>,
)> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    if !navigation_preload_sent {
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
    }
    Some((promise, resolver))
}

fn new_service_worker_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: &ServiceWorkerMessageEvent,
) -> Option<(&'static str, v8::Local<'s, v8::Object>)> {
    if !runtime_message_allowed_for_current_target(scope, &event.payload) {
        return None;
    }
    let (data, ports) = structured_deserialize_value_for_message_event(scope, &event.payload)?;
    Some((
        "message",
        new_service_worker_message_event_with_parts(scope, event, "message", data, ports),
    ))
}

fn new_service_worker_messageerror_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: &ServiceWorkerMessageEvent,
) -> v8::Local<'s, v8::Object> {
    let data = v8::null(scope).into();
    let ports = v8::Array::new(scope, 0);
    new_service_worker_message_event_with_parts(scope, event, "messageerror", data, ports)
}

fn new_service_worker_message_event_with_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: &ServiceWorkerMessageEvent,
    event_type: &'static str,
    data: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Object> {
    let source = if let Some(source_client) = event.source_client_snapshot.as_ref() {
        super::super::global_scope::build_service_worker_client_object_from_snapshot(
            scope,
            source_client,
        )
        .map(v8::Local::into)
        .unwrap_or_else(|| v8::null(scope).into())
    } else if let Some(source_worker) = event.source_worker.as_ref() {
        super::super::global_scope::build_service_worker_global_service_worker(scope, source_worker)
            .map(v8::Local::into)
            .unwrap_or_else(|_| v8::null(scope).into())
    } else {
        event
            .source_client_id
            .map(|client_id| {
                let url = event
                    .source_client_url
                    .as_ref()
                    .map(|url| url.as_str())
                    .unwrap_or_default();
                super::super::global_scope::build_service_worker_client_object(
                    scope,
                    client_id,
                    &crate::runtime::service_worker_exposed_client_id(client_id),
                    url,
                    "window",
                    "top-level",
                    "visible",
                    false,
                )
                .map(v8::Local::into)
            })
            .unwrap_or_else(|| Some(v8::null(scope).into()))
            .unwrap_or_else(|| v8::null(scope).into())
    };
    if let Some(event_object) = new_service_worker_extendable_message_event_with_parts(
        scope,
        event_type,
        data,
        source,
        ports,
        &event.source_origin,
    ) {
        install_service_worker_message_dispatch_methods(scope, event_object);
        return event_object;
    }
    ServiceWorkerMessageEventDeclaration {
        r#type: v8::String::new(scope, event_type).unwrap_or_else(|| v8::String::empty(scope)),
        data,
        source,
        origin: v8_string(scope, &event.source_origin).unwrap_or_else(|| v8::String::empty(scope)),
        ports,
        default_prevented: false,
        prevent_default: (),
        wait_until: (),
    }
    .bind(scope)
    .expect("service worker message event declaration should bind")
}

fn new_service_worker_extendable_message_event_with_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &'static str,
    data: v8::Local<'s, v8::Value>,
    source: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
    origin: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let constructor = global
        .get(scope, v8str(scope, "ExtendableMessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let event_type = v8::String::new(scope, event_type)?;
    let origin = v8_string(scope, origin)?;
    let last_event_id = v8::String::empty(scope);
    let init = ServiceWorkerExtendableMessageEventInitDeclaration::new(
        data,
        origin,
        last_event_id,
        source,
        ports,
    )
    .bind(scope)
    .ok()?;
    constructor.new_instance(scope, &[event_type.into(), init.into()])
}

fn install_service_worker_message_dispatch_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    let _ = ServiceWorkerMessageDispatchMethodsDeclaration::default().initialize(scope, event);
}

fn new_service_worker_notification_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _global: v8::Local<'s, v8::Object>,
    event: &ServiceWorkerNotificationEvent,
) -> Option<v8::Local<'s, v8::Object>> {
    let snapshot = crate::runtime::ServiceWorkerNotificationSnapshot {
        id: event.notification_id,
        registration_id: event.registration_id,
        title: event.title.clone(),
        tag: event.tag.clone(),
        metadata: event.metadata.clone(),
        actions: event.actions.clone(),
        data: event.data.clone(),
    };
    let notification =
        crate::context_bootstrap::build_notification_object_from_snapshot(scope, &snapshot)?;
    ServiceWorkerNotificationEventDeclaration {
        r#type: v8str(scope, event.kind.as_str()),
        notification,
        action: v8_string(scope, &event.action).unwrap_or_else(|| v8::String::empty(scope)),
        default_prevented: false,
        prevent_default: (),
        wait_until: (),
    }
    .bind(scope)
    .ok()
}

fn new_service_worker_push_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: &ServiceWorkerPushEvent,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = event
        .data
        .as_deref()
        .map(|bytes| new_service_worker_push_message_data(scope, bytes).map(v8::Local::into))
        .unwrap_or_else(|| Some(v8::null(scope).into()))?;
    ServiceWorkerPushEventDeclaration {
        r#type: v8str(scope, "push"),
        data,
        default_prevented: false,
        prevent_default: (),
        wait_until: (),
    }
    .bind(scope)
    .ok()
}

fn new_service_worker_push_message_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: &[u8],
) -> Option<v8::Local<'s, v8::Object>> {
    let object = ServiceWorkerPushMessageDataDeclaration {
        array_buffer: (),
        bytes: (),
        text: (),
        json: (),
    }
    .bind(scope)
    .ok()?;
    let buffer = array_buffer_from_bytes(scope, bytes.to_vec());
    set_private_value(
        scope,
        object,
        SERVICE_WORKER_PUSH_MESSAGE_DATA_BYTES_SLOT,
        buffer.into(),
    );
    Some(object)
}

fn push_message_data_bytes_from_this<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    this: v8::Local<'s, v8::Object>,
) -> Option<Vec<u8>> {
    let value = get_private_value(scope, this, SERVICE_WORKER_PUSH_MESSAGE_DATA_BYTES_SLOT)?;
    let buffer = v8::Local::<v8::ArrayBuffer>::try_from(value).ok()?;
    let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())?;
    let mut bytes = vec![0; view.byte_length()];
    let written = view.copy_contents(&mut bytes);
    bytes.truncate(written);
    Some(bytes)
}

fn array_buffer_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> v8::Local<'s, v8::ArrayBuffer> {
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    v8::ArrayBuffer::with_backing_store(scope, &backing_store)
}

fn uint8_array_from_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let len = bytes.len();
    let buffer = array_buffer_from_bytes(scope, bytes);
    v8::Uint8Array::new(scope, buffer, 0, len)
}

fn push_message_data_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes) = push_message_data_bytes_from_this(scope, args.this()) else {
        return;
    };
    let buffer = array_buffer_from_bytes(scope, bytes);
    rv.set(buffer.into());
}

fn push_message_data_bytes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes) = push_message_data_bytes_from_this(scope, args.this()) else {
        return;
    };
    let Some(array) = uint8_array_from_bytes(scope, bytes) else {
        return;
    };
    rv.set(array.into());
}

fn push_message_data_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes) = push_message_data_bytes_from_this(scope, args.this()) else {
        return;
    };
    let text = String::from_utf8_lossy(&bytes);
    let Some(value) = v8_string(scope, text.as_ref()) else {
        return;
    };
    rv.set(value.into());
}

fn push_message_data_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(bytes) = push_message_data_bytes_from_this(scope, args.this()) else {
        return;
    };
    let text = String::from_utf8_lossy(&bytes);
    let Some(source) = v8_string(scope, text.as_ref()) else {
        return;
    };
    let Some(value) = v8::json::parse(scope, source) else {
        let message = v8str(
            scope,
            "PushMessageData.json() could not parse the payload as JSON.",
        );
        let exception = v8::Exception::syntax_error(scope, message);
        scope.throw_exception(exception);
        return;
    };
    rv.set(value);
}

fn new_service_worker_sync_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: &ServiceWorkerSyncEvent,
) -> Option<v8::Local<'s, v8::Object>> {
    ServiceWorkerSyncEventDeclaration {
        r#type: v8str(scope, "sync"),
        tag: v8_string(scope, &event.tag).unwrap_or_else(|| v8::String::empty(scope)),
        last_chance: event.last_chance,
        default_prevented: false,
        prevent_default: (),
        wait_until: (),
    }
    .bind(scope)
    .ok()
}

fn new_service_worker_periodic_sync_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: &ServiceWorkerPeriodicSyncEvent,
) -> Option<v8::Local<'s, v8::Object>> {
    ServiceWorkerPeriodicSyncEventDeclaration {
        r#type: v8str(scope, "periodicsync"),
        tag: v8_string(scope, &event.tag).unwrap_or_else(|| v8::String::empty(scope)),
        default_prevented: false,
        prevent_default: (),
        wait_until: (),
    }
    .bind(scope)
    .ok()
}

fn new_service_worker_fetch_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    event: &ServiceWorkerFetchEvent,
) -> Option<(v8::Local<'s, v8::Object>, Option<u32>)> {
    let request_ctor = global
        .get(scope, v8str(scope, "Request").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let init = v8::Object::new(scope);
    set_object_string(scope, init, "method", &event.request.method);
    let constructible_mode = if event.request.request_mode == moli_fetch::RequestMode::Navigate {
        "same-origin"
    } else {
        event.request.request_mode.into()
    };
    set_object_string(scope, init, "mode", constructible_mode);
    set_object_string(
        scope,
        init,
        "credentials",
        event.request.credentials_mode.into(),
    );
    set_object_string(scope, init, "redirect", event.request.redirect_mode.into());
    set_object_string(scope, init, "cache", &event.request.metadata.cache);
    set_object_string(scope, init, "referrer", &event.request.metadata.referrer);
    set_object_string(
        scope,
        init,
        "referrerPolicy",
        &event.request.metadata.referrer_policy,
    );
    set_object_string(scope, init, "integrity", &event.request.metadata.integrity);
    set_object_bool(scope, init, "keepalive", event.request.metadata.keepalive);
    if let Some(priority) = event.request.priority {
        let priority: &'static str = priority.into();
        set_object_string(scope, init, "priority", priority);
    }
    let headers = v8::Object::new(scope);
    for (name, value) in &event.request.headers {
        set_object_string(scope, headers, name, value);
    }
    let _ = init.set(scope, v8str(scope, "headers").into(), headers.into());
    if !matches!(event.request.method.as_str(), "GET" | "HEAD")
        && let Some(body) = event.request.body.clone()
    {
        let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(body).make_shared();
        let body = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
        let _ = init.set(scope, v8str(scope, "body").into(), body.into());
    }
    let url = v8::String::new(scope, event.request.url.as_str())?;
    let request = request_ctor.new_instance(scope, &[url.into(), init.into()])?;
    set_request_destination_for_service_worker_fetch_event(
        scope,
        request,
        event.request.destination.as_str(),
    );
    set_request_mode_for_service_worker_fetch_event(
        scope,
        request,
        event.request.request_mode.into(),
    );
    set_request_reload_navigation_for_service_worker_fetch_event(
        scope,
        request,
        event.request.is_reload,
    );
    let request_signal_id = request
        .get(scope, v8str(scope, "signal").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|signal| worker_abort_signal_id(scope, signal));
    Some((request, request_signal_id))
}

fn set_object_string(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
    value: &str,
) {
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    let Some(key) = v8::String::new(scope, key) else {
        return;
    };
    let _ = object.set(scope, key.into(), value.into());
}

fn set_object_bool(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
    value: bool,
) {
    let Some(key) = v8::String::new(scope, key) else {
        return;
    };
    let _ = object.set(scope, key.into(), v8::Boolean::new(scope, value).into());
}

fn set_service_worker_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_id: ServiceWorkerEventId,
) {
    set_private_value(
        scope,
        event,
        SERVICE_WORKER_LIFECYCLE_EVENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, event_id.as_u64()).into(),
    );
}

fn service_worker_lifecycle_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerEventId> {
    let value = get_private_value(scope, event, SERVICE_WORKER_LIFECYCLE_EVENT_ID_SLOT)?;
    let id = service_worker_event_id_value(scope, value)?;
    Some(ServiceWorkerEventId::from_u64_for_worker(id))
}

fn set_service_worker_fetch_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_id: ServiceWorkerEventId,
) {
    set_private_value(
        scope,
        event,
        SERVICE_WORKER_FETCH_EVENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, event_id.as_u64()).into(),
    );
}

fn service_worker_fetch_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerEventId> {
    let value = get_private_value(scope, event, SERVICE_WORKER_FETCH_EVENT_ID_SLOT)?;
    let id = service_worker_event_id_value(scope, value)?;
    Some(ServiceWorkerEventId::from_u64_for_worker(id))
}

fn set_service_worker_message_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_id: ServiceWorkerEventId,
) {
    set_private_value(
        scope,
        event,
        SERVICE_WORKER_MESSAGE_EVENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, event_id.as_u64()).into(),
    );
}

fn service_worker_message_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerEventId> {
    let value = get_private_value(scope, event, SERVICE_WORKER_MESSAGE_EVENT_ID_SLOT)?;
    let id = service_worker_event_id_value(scope, value)?;
    Some(ServiceWorkerEventId::from_u64_for_worker(id))
}

fn set_service_worker_notification_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_id: ServiceWorkerEventId,
) {
    set_private_value(
        scope,
        event,
        SERVICE_WORKER_NOTIFICATION_EVENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, event_id.as_u64()).into(),
    );
}

fn service_worker_notification_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerEventId> {
    let value = get_private_value(scope, event, SERVICE_WORKER_NOTIFICATION_EVENT_ID_SLOT)?;
    let id = service_worker_event_id_value(scope, value)?;
    Some(ServiceWorkerEventId::from_u64_for_worker(id))
}

fn set_service_worker_push_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_id: ServiceWorkerEventId,
) {
    set_private_value(
        scope,
        event,
        SERVICE_WORKER_PUSH_EVENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, event_id.as_u64()).into(),
    );
}

fn service_worker_push_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerEventId> {
    let value = get_private_value(scope, event, SERVICE_WORKER_PUSH_EVENT_ID_SLOT)?;
    let id = service_worker_event_id_value(scope, value)?;
    Some(ServiceWorkerEventId::from_u64_for_worker(id))
}

fn set_service_worker_sync_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_id: ServiceWorkerEventId,
) {
    set_private_value(
        scope,
        event,
        SERVICE_WORKER_SYNC_EVENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, event_id.as_u64()).into(),
    );
}

fn service_worker_sync_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerEventId> {
    let value = get_private_value(scope, event, SERVICE_WORKER_SYNC_EVENT_ID_SLOT)?;
    let id = service_worker_event_id_value(scope, value)?;
    Some(ServiceWorkerEventId::from_u64_for_worker(id))
}

fn set_service_worker_periodic_sync_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_id: ServiceWorkerEventId,
) {
    set_private_value(
        scope,
        event,
        SERVICE_WORKER_PERIODIC_SYNC_EVENT_ID_SLOT,
        v8::BigInt::new_from_u64(scope, event_id.as_u64()).into(),
    );
}

fn service_worker_periodic_sync_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<ServiceWorkerEventId> {
    let value = get_private_value(scope, event, SERVICE_WORKER_PERIODIC_SYNC_EVENT_ID_SLOT)?;
    let id = service_worker_event_id_value(scope, value)?;
    Some(ServiceWorkerEventId::from_u64_for_worker(id))
}

fn service_worker_event_id_value(
    _scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<u64> {
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, lossless) = big.u64_value();
    lossless.then_some(id)
}

fn service_worker_wait_until_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    let lifecycle_event_id = service_worker_lifecycle_event_id(scope, event);
    let fetch_event_id = service_worker_fetch_event_id(scope, event);
    let message_event_id = service_worker_message_event_id(scope, event);
    let notification_event_id = service_worker_notification_event_id(scope, event);
    let push_event_id = service_worker_push_event_id(scope, event);
    let sync_event_id = service_worker_sync_event_id(scope, event);
    let periodic_sync_event_id = service_worker_periodic_sync_event_id(scope, event);
    let Some((event_id, event_kind)) = lifecycle_event_id
        .map(|event_id| (event_id, ServiceWorkerWaitUntilEventKind::Lifecycle))
        .or_else(|| {
            fetch_event_id.map(|event_id| (event_id, ServiceWorkerWaitUntilEventKind::Fetch))
        })
        .or_else(|| {
            message_event_id.map(|event_id| (event_id, ServiceWorkerWaitUntilEventKind::Message))
        })
        .or_else(|| {
            notification_event_id
                .map(|event_id| (event_id, ServiceWorkerWaitUntilEventKind::Notification))
        })
        .or_else(|| push_event_id.map(|event_id| (event_id, ServiceWorkerWaitUntilEventKind::Push)))
        .or_else(|| sync_event_id.map(|event_id| (event_id, ServiceWorkerWaitUntilEventKind::Sync)))
        .or_else(|| {
            periodic_sync_event_id
                .map(|event_id| (event_id, ServiceWorkerWaitUntilEventKind::PeriodicSync))
        })
    else {
        throw_service_worker_wait_until_invalid_state(scope);
        return;
    };
    let is_event_active = get_worker_state(scope).is_some_and(|state| {
        let state = state.borrow();
        match event_kind {
            ServiceWorkerWaitUntilEventKind::Lifecycle => state
                .pending_service_worker_lifecycle_events
                .get(&event_id)
                .is_some_and(|pending| {
                    !pending.dispatch_finished || pending.pending_wait_until_count > 0
                }),
            ServiceWorkerWaitUntilEventKind::Fetch => state
                .pending_service_worker_fetch_events
                .get(&event_id)
                .is_some_and(|pending| {
                    !pending.dispatch_finished || pending.pending_wait_until_count > 0
                }),
            ServiceWorkerWaitUntilEventKind::Message => state
                .pending_service_worker_message_events
                .get(&event_id)
                .is_some_and(|pending| {
                    !pending.dispatch_finished || pending.pending_wait_until_count > 0
                }),
            ServiceWorkerWaitUntilEventKind::Notification => state
                .pending_service_worker_notification_events
                .get(&event_id)
                .is_some_and(|pending| {
                    !pending.dispatch_finished || pending.pending_wait_until_count > 0
                }),
            ServiceWorkerWaitUntilEventKind::Push => state
                .pending_service_worker_push_events
                .get(&event_id)
                .is_some_and(|pending| {
                    !pending.dispatch_finished || pending.pending_wait_until_count > 0
                }),
            ServiceWorkerWaitUntilEventKind::Sync => state
                .pending_service_worker_sync_events
                .get(&event_id)
                .is_some_and(|pending| {
                    !pending.dispatch_finished || pending.pending_wait_until_count > 0
                }),
            ServiceWorkerWaitUntilEventKind::PeriodicSync => state
                .pending_service_worker_periodic_sync_events
                .get(&event_id)
                .is_some_and(|pending| {
                    !pending.dispatch_finished || pending.pending_wait_until_count > 0
                }),
        }
    });
    if !is_event_active {
        throw_service_worker_wait_until_invalid_state(scope);
        return;
    }
    let Some(on_fulfilled) = build_service_worker_wait_until_callback(
        scope,
        event_id,
        event_kind,
        true,
        service_worker_wait_until_fulfilled_callback,
    ) else {
        return;
    };
    let Some(on_rejected) = build_service_worker_wait_until_callback(
        scope,
        event_id,
        event_kind,
        false,
        service_worker_wait_until_rejected_callback,
    ) else {
        return;
    };
    let did_register = if let Some(state) = get_worker_state(scope) {
        let mut state = state.borrow_mut();
        match event_kind {
            ServiceWorkerWaitUntilEventKind::Lifecycle => {
                if let Some(pending) = state
                    .pending_service_worker_lifecycle_events
                    .get_mut(&event_id)
                {
                    pending.pending_wait_until_count += 1;
                    true
                } else {
                    false
                }
            }
            ServiceWorkerWaitUntilEventKind::Fetch => {
                if let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id)
                {
                    pending.pending_wait_until_count += 1;
                    true
                } else {
                    false
                }
            }
            ServiceWorkerWaitUntilEventKind::Message => {
                if let Some(pending) = state
                    .pending_service_worker_message_events
                    .get_mut(&event_id)
                {
                    pending.pending_wait_until_count += 1;
                    true
                } else {
                    false
                }
            }
            ServiceWorkerWaitUntilEventKind::Notification => {
                if let Some(pending) = state
                    .pending_service_worker_notification_events
                    .get_mut(&event_id)
                {
                    pending.pending_wait_until_count += 1;
                    true
                } else {
                    false
                }
            }
            ServiceWorkerWaitUntilEventKind::Push => {
                if let Some(pending) = state.pending_service_worker_push_events.get_mut(&event_id) {
                    pending.pending_wait_until_count += 1;
                    true
                } else {
                    false
                }
            }
            ServiceWorkerWaitUntilEventKind::Sync => {
                if let Some(pending) = state.pending_service_worker_sync_events.get_mut(&event_id) {
                    pending.pending_wait_until_count += 1;
                    true
                } else {
                    false
                }
            }
            ServiceWorkerWaitUntilEventKind::PeriodicSync => {
                if let Some(pending) = state
                    .pending_service_worker_periodic_sync_events
                    .get_mut(&event_id)
                {
                    pending.pending_wait_until_count += 1;
                    true
                } else {
                    false
                }
            }
        }
    } else {
        false
    };
    if !did_register {
        throw_service_worker_wait_until_invalid_state(scope);
        return;
    }
    if !promise_resolve_then2_after_wait_until_microtask(
        scope,
        args.get(0),
        on_fulfilled,
        on_rejected,
    ) {
        service_worker_wait_until_settled_for_event(scope, event_id, event_kind, false);
    }
}

fn throw_service_worker_wait_until_invalid_state(scope: &mut v8::PinScope<'_, '_>) {
    let exception = crate::context_bootstrap::new_dom_exception_value(
        scope,
        "The event handler is already finished and no extend lifetime promises are outstanding.",
        "InvalidStateError",
    );
    scope.throw_exception(exception);
}

fn promise_resolve_then2<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    on_fulfilled: v8::Local<'s, v8::Function>,
    on_rejected: v8::Local<'s, v8::Function>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let promise = promise_resolve_value(scope, value)?;
    promise.then2(scope, on_fulfilled, on_rejected)
}

fn promise_resolve_then2_after_wait_until_microtask<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    on_fulfilled: v8::Local<'s, v8::Function>,
    on_rejected: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(extra_fulfilled) =
        v8::Function::builder(service_worker_wait_until_extra_fulfilled_callback).build(scope)
    else {
        return false;
    };
    let Some(extra_rejected) =
        v8::Function::builder(service_worker_wait_until_extra_rejected_callback).build(scope)
    else {
        return false;
    };
    let Some(promise) = promise_resolve_value(scope, value) else {
        return false;
    };
    let Some(wait_until_promise) = promise.then2(scope, extra_fulfilled, extra_rejected) else {
        return false;
    };
    wait_until_promise
        .then2(scope, on_fulfilled, on_rejected)
        .is_some()
}

fn promise_resolve_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let promise = if let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) {
        promise
    } else {
        let resolver = v8::PromiseResolver::new(scope)?;
        let promise = resolver.get_promise(scope);
        if resolver.resolve(scope, value) != Some(true) {
            return None;
        }
        promise
    };
    Some(promise)
}

fn service_worker_wait_until_extra_fulfilled_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

fn service_worker_wait_until_extra_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    scope.throw_exception(args.get(0));
}

fn build_service_worker_wait_until_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_id: ServiceWorkerEventId,
    event_kind: ServiceWorkerWaitUntilEventKind,
    fulfilled: bool,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Option<v8::Local<'s, v8::Function>> {
    let data = ServiceWorkerWaitUntilCallbackDataDeclaration {
        event_id: v8::BigInt::new_from_u64(scope, event_id.as_u64()),
        event_kind: v8::String::new(scope, service_worker_wait_until_event_kind_name(event_kind))?,
        fulfilled,
    }
    .bind(scope)
    .ok()?;
    v8::Function::builder(callback)
        .data(data.into())
        .build(scope)
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ServiceWorkerWaitUntilCallbackDataDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    event_id: v8::Local<'scope, v8::BigInt>,
    #[webapi(data_property, enumerable)]
    event_kind: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    fulfilled: bool,
}

fn service_worker_wait_until_event_kind_name(
    event_kind: ServiceWorkerWaitUntilEventKind,
) -> &'static str {
    match event_kind {
        ServiceWorkerWaitUntilEventKind::Lifecycle => "lifecycle",
        ServiceWorkerWaitUntilEventKind::Fetch => "fetch",
        ServiceWorkerWaitUntilEventKind::Message => "message",
        ServiceWorkerWaitUntilEventKind::Notification => "notification",
        ServiceWorkerWaitUntilEventKind::Push => "push",
        ServiceWorkerWaitUntilEventKind::Sync => "sync",
        ServiceWorkerWaitUntilEventKind::PeriodicSync => "periodicsync",
    }
}

fn service_worker_wait_until_event_kind_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<ServiceWorkerWaitUntilEventKind> {
    match value.to_string(scope)?.to_rust_string_lossy(scope).as_str() {
        "lifecycle" => Some(ServiceWorkerWaitUntilEventKind::Lifecycle),
        "fetch" => Some(ServiceWorkerWaitUntilEventKind::Fetch),
        "message" => Some(ServiceWorkerWaitUntilEventKind::Message),
        "notification" => Some(ServiceWorkerWaitUntilEventKind::Notification),
        "push" => Some(ServiceWorkerWaitUntilEventKind::Push),
        "sync" => Some(ServiceWorkerWaitUntilEventKind::Sync),
        "periodicsync" => Some(ServiceWorkerWaitUntilEventKind::PeriodicSync),
        _ => None,
    }
}

fn service_worker_wait_until_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    service_worker_wait_until_settled(scope, &args, true);
}

fn service_worker_wait_until_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    service_worker_wait_until_settled(scope, &args, false);
}

fn service_worker_wait_until_settled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    fulfilled: bool,
) {
    let Some(data) = v8::Local::<v8::Object>::try_from(args.data()).ok() else {
        return;
    };
    let Some(event_id) = data
        .get(scope, v8str(scope, "eventId").into())
        .and_then(|value| service_worker_event_id_value(scope, value))
        .map(ServiceWorkerEventId::from_u64_for_worker)
    else {
        return;
    };
    let Some(event_kind) = data
        .get(scope, v8str(scope, "eventKind").into())
        .and_then(|value| service_worker_wait_until_event_kind_from_value(scope, value))
    else {
        return;
    };
    service_worker_wait_until_settled_for_event(scope, event_id, event_kind, fulfilled);
}

fn service_worker_wait_until_settled_for_event(
    scope: &mut v8::PinScope<'_, '_>,
    event_id: ServiceWorkerEventId,
    event_kind: ServiceWorkerWaitUntilEventKind,
    fulfilled: bool,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    {
        let mut state = state.borrow_mut();
        match event_kind {
            ServiceWorkerWaitUntilEventKind::Lifecycle => {
                let Some(pending) = state
                    .pending_service_worker_lifecycle_events
                    .get_mut(&event_id)
                else {
                    return;
                };
                pending.pending_wait_until_count =
                    pending.pending_wait_until_count.saturating_sub(1);
                if !fulfilled {
                    pending.completion.result =
                        Err("service worker waitUntil promise rejected".to_owned());
                }
            }
            ServiceWorkerWaitUntilEventKind::Fetch => {
                let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id)
                else {
                    return;
                };
                pending.pending_wait_until_count =
                    pending.pending_wait_until_count.saturating_sub(1);
            }
            ServiceWorkerWaitUntilEventKind::Message => {
                let Some(pending) = state
                    .pending_service_worker_message_events
                    .get_mut(&event_id)
                else {
                    return;
                };
                pending.pending_wait_until_count =
                    pending.pending_wait_until_count.saturating_sub(1);
                if !fulfilled {
                    pending.completion.result =
                        Err("service worker waitUntil promise rejected".to_owned());
                }
            }
            ServiceWorkerWaitUntilEventKind::Notification => {
                let Some(pending) = state
                    .pending_service_worker_notification_events
                    .get_mut(&event_id)
                else {
                    return;
                };
                pending.pending_wait_until_count =
                    pending.pending_wait_until_count.saturating_sub(1);
                if !fulfilled {
                    pending.completion.result =
                        Err("service worker waitUntil promise rejected".to_owned());
                }
            }
            ServiceWorkerWaitUntilEventKind::Push => {
                let Some(pending) = state.pending_service_worker_push_events.get_mut(&event_id)
                else {
                    return;
                };
                pending.pending_wait_until_count =
                    pending.pending_wait_until_count.saturating_sub(1);
                if !fulfilled {
                    pending.completion.result =
                        Err("service worker waitUntil promise rejected".to_owned());
                }
            }
            ServiceWorkerWaitUntilEventKind::Sync => {
                let Some(pending) = state.pending_service_worker_sync_events.get_mut(&event_id)
                else {
                    return;
                };
                pending.pending_wait_until_count =
                    pending.pending_wait_until_count.saturating_sub(1);
                if !fulfilled {
                    pending.completion.result =
                        Err("service worker waitUntil promise rejected".to_owned());
                }
            }
            ServiceWorkerWaitUntilEventKind::PeriodicSync => {
                let Some(pending) = state
                    .pending_service_worker_periodic_sync_events
                    .get_mut(&event_id)
                else {
                    return;
                };
                pending.pending_wait_until_count =
                    pending.pending_wait_until_count.saturating_sub(1);
                if !fulfilled {
                    pending.completion.result =
                        Err("service worker waitUntil promise rejected".to_owned());
                }
            }
        }
    }
    match event_kind {
        ServiceWorkerWaitUntilEventKind::Lifecycle => {
            maybe_send_service_worker_lifecycle_completion(&state, event_id);
        }
        ServiceWorkerWaitUntilEventKind::Fetch => {
            maybe_send_service_worker_fetch_completion(&state, event_id, scope);
        }
        ServiceWorkerWaitUntilEventKind::Message => {
            maybe_send_service_worker_message_completion(&state, event_id);
        }
        ServiceWorkerWaitUntilEventKind::Notification => {
            maybe_send_service_worker_notification_completion(&state, event_id);
        }
        ServiceWorkerWaitUntilEventKind::Push => {
            maybe_send_service_worker_push_completion(&state, event_id);
        }
        ServiceWorkerWaitUntilEventKind::Sync => {
            maybe_send_service_worker_sync_completion(&state, event_id);
        }
        ServiceWorkerWaitUntilEventKind::PeriodicSync => {
            maybe_send_service_worker_periodic_sync_completion(&state, event_id);
        }
    }
}

fn service_worker_respond_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    let Some(event_id) = service_worker_fetch_event_id(scope, event) else {
        throw_service_worker_wait_until_invalid_state(scope);
        return;
    };
    let is_event_active = get_worker_state(scope).is_some_and(|state| {
        let state = state.borrow();
        state
            .pending_service_worker_fetch_events
            .get(&event_id)
            .is_some_and(|pending| !pending.dispatch_finished && !pending.respond_with_called)
    });
    if !is_event_active {
        throw_service_worker_wait_until_invalid_state(scope);
        return;
    }
    let Some(on_fulfilled) = build_service_worker_respond_with_callback(
        scope,
        event_id,
        true,
        service_worker_respond_with_fulfilled_callback,
    ) else {
        return;
    };
    let Some(on_rejected) = build_service_worker_respond_with_callback(
        scope,
        event_id,
        false,
        service_worker_respond_with_rejected_callback,
    ) else {
        return;
    };
    let Some(on_lifetime_fulfilled) = build_service_worker_respond_with_lifetime_callback(
        scope,
        event_id,
        service_worker_respond_with_lifetime_fulfilled_callback,
    ) else {
        return;
    };
    let Some(on_lifetime_rejected) = build_service_worker_respond_with_lifetime_callback(
        scope,
        event_id,
        service_worker_respond_with_lifetime_rejected_callback,
    ) else {
        return;
    };
    let did_register = if let Some(state) = get_worker_state(scope) {
        let mut state = state.borrow_mut();
        if let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) {
            pending.respond_with_called = true;
            pending.pending_respond_with = true;
            pending.pending_wait_until_count += 1;
            true
        } else {
            false
        }
    } else {
        false
    };
    if !did_register {
        throw_service_worker_wait_until_invalid_state(scope);
        return;
    }
    set_event_stop_immediate_propagation(scope, event);
    let Some(settlement_promise) =
        promise_resolve_then2(scope, args.get(0), on_fulfilled, on_rejected)
    else {
        service_worker_respond_with_settled_for_event(
            scope,
            event_id,
            ServiceWorkerFetchResult::Failure(
                "FetchEvent.respondWith failed to attach promise reactions".to_owned(),
            ),
        );
        service_worker_respond_with_lifetime_settled_for_event(scope, event_id);
        return;
    };
    if settlement_promise
        .then2(scope, on_lifetime_fulfilled, on_lifetime_rejected)
        .is_none()
    {
        service_worker_respond_with_lifetime_settled_for_event(scope, event_id);
    }
}

fn build_service_worker_respond_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_id: ServiceWorkerEventId,
    fulfilled: bool,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Option<v8::Local<'s, v8::Function>> {
    let data = ServiceWorkerRespondWithCallbackDataDeclaration {
        event_id: v8::BigInt::new_from_u64(scope, event_id.as_u64()),
        fulfilled,
    }
    .bind(scope)
    .ok()?;
    v8::Function::builder(callback)
        .data(data.into())
        .build(scope)
}

fn service_worker_respond_with_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    service_worker_respond_with_settled(scope, &args, true);
}

fn service_worker_respond_with_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    service_worker_respond_with_settled(scope, &args, false);
}

fn service_worker_respond_with_settled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    fulfilled: bool,
) {
    let Some(data) = v8::Local::<v8::Object>::try_from(args.data()).ok() else {
        return;
    };
    let Some(event_id) = data
        .get(scope, v8str(scope, "eventId").into())
        .and_then(|value| service_worker_event_id_value(scope, value))
        .map(ServiceWorkerEventId::from_u64_for_worker)
    else {
        return;
    };
    let result = if fulfilled {
        match materialize_response_object_head_for_service_worker_respond_with(
            scope,
            args.get(0),
            "FetchEvent.respondWith",
        ) {
            Ok((head, response)) => {
                if let Some(error) =
                    service_worker_respond_with_response_head_rejection(scope, event_id, &head)
                {
                    service_worker_respond_with_settled_for_event(
                        scope,
                        event_id,
                        ServiceWorkerFetchResult::Failure(error),
                    );
                    return;
                }
                let body_source_id = new_network_body_source_id();
                let (body, stream_cancel_handle) =
                    build_service_worker_respond_with_stream_chunk_callback(
                        scope,
                        event_id,
                        body_source_id,
                    )
                    .map(|callback| {
                        materialize_response_object_body_with_chunk_callback(
                            scope,
                            response,
                            "FetchEvent.respondWith",
                            callback,
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            materialize_response_object_body(
                                scope,
                                response,
                                "FetchEvent.respondWith",
                            ),
                            None,
                        )
                    });
                match body {
                    MaterializedResponseBody::Ready(body) => ServiceWorkerFetchResult::Response(
                        service_worker_fetch_response_from_materialized(head, body),
                    ),
                    MaterializedResponseBody::Pending(promise) => {
                        let stream_body = stream_cancel_handle.is_some();
                        let started_head = stream_body
                            .then(|| service_worker_fetch_response_head_from_materialized(&head));
                        let stream_cancel_handle =
                            stream_cancel_handle.map(|handle| (body_source_id, handle));
                        if service_worker_respond_with_pending_body(
                            scope,
                            event_id,
                            head,
                            promise,
                            stream_cancel_handle,
                        ) {
                            if stream_body {
                                resolve_service_worker_fetch_handled_for_event(scope, event_id);
                            }
                            if let Some(started_head) = started_head {
                                send_service_worker_fetch_stream_started(
                                    scope,
                                    event_id,
                                    body_source_id,
                                    started_head,
                                );
                            }
                            return;
                        }
                        ServiceWorkerFetchResult::Failure(
                            "FetchEvent.respondWith failed to attach response body reactions"
                                .to_owned(),
                        )
                    }
                    MaterializedResponseBody::Failure(error) => {
                        ServiceWorkerFetchResult::Failure(error)
                    }
                }
            }
            Err(error) => ServiceWorkerFetchResult::Failure(error),
        }
    } else {
        ServiceWorkerFetchResult::Failure(format!(
            "FetchEvent.respondWith promise rejected: {}",
            js_value_error_string(scope, args.get(0))
        ))
    };
    service_worker_respond_with_settled_for_event(scope, event_id, result);
}

fn service_worker_respond_with_response_head_rejection(
    scope: &mut v8::PinScope<'_, '_>,
    event_id: ServiceWorkerEventId,
    head: &MaterializedResponseHead,
) -> Option<String> {
    if head.response_type != "opaque" {
        return None;
    }
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    let pending = state.pending_service_worker_fetch_events.get(&event_id)?;
    crate::service_worker_runtime::service_worker_opaque_response_rejection(
        pending.request_mode,
        pending.request_destination,
    )
}

fn build_service_worker_respond_with_lifetime_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_id: ServiceWorkerEventId,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Option<v8::Local<'s, v8::Function>> {
    let data = ServiceWorkerRespondWithLifetimeCallbackDataDeclaration {
        event_id: v8::BigInt::new_from_u64(scope, event_id.as_u64()),
    }
    .bind(scope)
    .ok()?;
    v8::Function::builder(callback)
        .data(data.into())
        .build(scope)
}

fn service_worker_respond_with_lifetime_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    service_worker_respond_with_lifetime_settled(scope, &args);
}

fn service_worker_respond_with_lifetime_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    service_worker_respond_with_lifetime_settled(scope, &args);
}

fn service_worker_respond_with_lifetime_settled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) {
    let Some(data) = v8::Local::<v8::Object>::try_from(args.data()).ok() else {
        return;
    };
    let Some(event_id) = data
        .get(scope, v8str(scope, "eventId").into())
        .and_then(|value| service_worker_event_id_value(scope, value))
        .map(ServiceWorkerEventId::from_u64_for_worker)
    else {
        return;
    };
    service_worker_respond_with_lifetime_settled_for_event(scope, event_id);
}

fn service_worker_respond_with_lifetime_settled_for_event(
    scope: &mut v8::PinScope<'_, '_>,
    event_id: ServiceWorkerEventId,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) else {
            return;
        };
        pending.pending_wait_until_count = pending.pending_wait_until_count.saturating_sub(1);
    }
    maybe_send_service_worker_fetch_completion(&state, event_id, scope);
}

fn service_worker_fetch_response_from_materialized(
    head: MaterializedResponseHead,
    body: Vec<u8>,
) -> ServiceWorkerFetchResponse {
    let response = head.with_body(body);
    ServiceWorkerFetchResponse {
        final_url: response.final_url,
        response_type: response.response_type,
        redirected: response.redirected,
        status: response.status,
        status_text: response.status_text,
        headers: response.headers,
        body: response.body,
    }
}

fn service_worker_fetch_response_head_from_materialized(
    head: &MaterializedResponseHead,
) -> MaterializedServiceWorkerFetchResponseHead {
    MaterializedServiceWorkerFetchResponseHead {
        final_url: head.final_url.clone(),
        response_type: head.response_type.clone(),
        redirected: head.redirected,
        status: head.status,
        headers: head.headers.clone(),
    }
}

fn send_service_worker_fetch_stream_started(
    scope: &mut v8::PinScope<'_, '_>,
    event_id: ServiceWorkerEventId,
    body_source_id: NetworkBodySourceId,
    response_head: MaterializedServiceWorkerFetchResponseHead,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let Some((parent_tx, version_id, run)) = ({
        let state = state.borrow();
        state
            .pending_service_worker_fetch_events
            .get(&event_id)
            .map(|pending| {
                (
                    state.parent_tx.clone(),
                    pending.completion.version_id,
                    pending.completion.run.clone(),
                )
            })
    }) else {
        return;
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerFetchStreamStarted(
        ServiceWorkerFetchStreamStarted {
            event_id,
            version_id,
            run,
            body_source_id,
            response_head,
        },
    ));
}

fn service_worker_respond_with_pending_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_id: ServiceWorkerEventId,
    head: MaterializedResponseHead,
    promise: v8::Local<'s, v8::Promise>,
    stream_cancel_handle: Option<(NetworkBodySourceId, v8::Global<v8::Object>)>,
) -> bool {
    let Some(on_fulfilled) = build_service_worker_respond_with_body_callback(
        scope,
        event_id,
        service_worker_respond_with_body_fulfilled_callback,
    ) else {
        return false;
    };
    let Some(on_rejected) = build_service_worker_respond_with_body_callback(
        scope,
        event_id,
        service_worker_respond_with_body_rejected_callback,
    ) else {
        return false;
    };
    let Some(state) = get_worker_state(scope) else {
        return true;
    };
    {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) else {
            return true;
        };
        pending.pending_respond_with_response = Some(head);
        if let Some((body_source_id, cancel_handle)) = stream_cancel_handle {
            pending.pending_respond_with_stream_body_source_id = Some(body_source_id);
            pending.pending_respond_with_stream_cancel_handle = Some(cancel_handle);
        } else {
            pending.pending_respond_with_stream_body_source_id = None;
            pending.pending_respond_with_stream_cancel_handle = None;
        }
    }
    if promise.then2(scope, on_fulfilled, on_rejected).is_some() {
        return true;
    }
    if let Some(state) = get_worker_state(scope)
        && let Some(pending) = state
            .borrow_mut()
            .pending_service_worker_fetch_events
            .get_mut(&event_id)
    {
        pending.pending_respond_with_response = None;
        pending.pending_respond_with_stream_body_source_id = None;
        pending.pending_respond_with_stream_cancel_handle = None;
    }
    false
}

fn cancel_service_worker_fetch_stream_in_context(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
    body_source_id: NetworkBodySourceId,
) {
    let cancel_handle = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) else {
            return;
        };
        if pending.pending_respond_with_stream_body_source_id != Some(body_source_id) {
            return;
        }
        pending.pending_respond_with_stream_body_source_id = None;
        pending.pending_respond_with_stream_cancel_handle.take()
    };
    let Some(cancel_handle) = cancel_handle else {
        return;
    };
    let stream = v8::Local::new(scope, &cancel_handle);
    let reason = "The operation was aborted.";
    let reason_value: v8::Local<'_, v8::Value> = match v8_string(scope, reason) {
        Some(reason) => reason.into(),
        None => v8::undefined(scope).into(),
    };
    crate::context_bootstrap::cancel_readable_stream(scope, stream, reason_value);
    service_worker_respond_with_body_settled_for_event(
        scope,
        event_id,
        Err(format!(
            "FetchEvent.respondWith stream body was canceled: {reason}"
        )),
    );
}

fn abort_service_worker_fetch_request_signal_in_context(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
    reason_payload: Option<V8StructuredClonePayload>,
) {
    let Some(signal_id) = state
        .borrow()
        .pending_service_worker_fetch_events
        .get(&event_id)
        .and_then(|pending| pending.request_signal_id)
    else {
        return;
    };
    let reason = reason_payload
        .as_ref()
        .and_then(|payload| {
            structured_deserialize_value_for_message_event(scope, payload).map(|(value, _)| value)
        })
        .unwrap_or_else(|| worker_abort_error_value(scope));
    abort_worker_signal_by_id(scope, signal_id, reason);
}

pub(super) fn start_service_worker_navigation_preload_response(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    started: ServiceWorkerNavigationPreloadResponseStarted,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    start_service_worker_navigation_preload_response_in_context(scope, state, started);
}

fn start_service_worker_navigation_preload_response_in_context(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    started: ServiceWorkerNavigationPreloadResponseStarted,
) {
    let Some(resolver) = ({
        let mut state = state.borrow_mut();
        let Some(preload) = state
            .pending_service_worker_navigation_preloads
            .get_mut(&started.event_id)
        else {
            return;
        };
        if preload.version_id != started.version_id
            || preload.run != started.run
            || preload.body_source_id.is_some()
        {
            return;
        }
        preload.body_source_id = Some(started.body_source_id);
        preload.resolver.take()
    }) else {
        return;
    };

    let head = moli_fetch::ResponseHead {
        final_url: started
            .response_head
            .final_url
            .clone()
            .unwrap_or_else(|| started.request_url.clone()),
        status: started.response_head.status,
        headers: started.response_head.headers.clone(),
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        redirected: started.response_head.redirected,
        redirect_chain: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    };
    let response_object = build_navigation_preload_response_object_from_stream_for_request_mode(
        scope,
        &started.request_url,
        started.request_mode,
        head,
        started.body_source_id,
    );
    let resolver = v8::Local::new(scope, resolver);
    let _ = resolver.resolve(scope, response_object.into());
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn enqueue_service_worker_navigation_preload_stream_chunk(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    chunk: ServiceWorkerNavigationPreloadStreamChunk,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    if !service_worker_navigation_preload_chunk_matches_pending_event(state, &chunk) {
        return;
    }
    enqueue_pending_network_body_chunk(scope, chunk.body_source_id, chunk.bytes);
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

fn service_worker_navigation_preload_chunk_matches_pending_event(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    chunk: &ServiceWorkerNavigationPreloadStreamChunk,
) -> bool {
    state
        .borrow()
        .pending_service_worker_navigation_preloads
        .get(&chunk.event_id)
        .is_some_and(|preload| preload.body_source_id == Some(chunk.body_source_id))
}

pub(super) fn finish_service_worker_navigation_preload_stream(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    finished: ServiceWorkerNavigationPreloadStreamFinished,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    if !service_worker_navigation_preload_finish_matches_pending_event(state, &finished) {
        return;
    }
    match finished.result {
        Ok(()) => {
            close_pending_network_body_stream(scope, finished.body_source_id);
        }
        Err(message) => {
            let reason = v8_string(scope, &message)
                .map(|message| v8::Exception::type_error(scope, message))
                .unwrap_or_else(|| v8::undefined(scope).into());
            error_pending_network_body_stream_with_reason(
                scope,
                finished.body_source_id,
                message,
                reason,
            );
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    state
        .borrow_mut()
        .pending_service_worker_navigation_preloads
        .remove(&finished.event_id);
}

fn service_worker_navigation_preload_finish_matches_pending_event(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    finished: &ServiceWorkerNavigationPreloadStreamFinished,
) -> bool {
    let state = state.borrow();
    let Some(preload) = state
        .pending_service_worker_navigation_preloads
        .get(&finished.event_id)
    else {
        return false;
    };
    if preload.version_id != finished.version_id
        || preload.run != finished.run
        || preload.body_source_id != Some(finished.body_source_id)
    {
        return false;
    }
    true
}

pub(super) fn fail_service_worker_navigation_preload_response(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    failure: ServiceWorkerNavigationPreloadFailure,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    fail_service_worker_navigation_preload_response_in_context(scope, state, failure);
}

fn fail_service_worker_navigation_preload_response_in_context(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    failure: ServiceWorkerNavigationPreloadFailure,
) {
    let Some(resolver) = ({
        let mut state = state.borrow_mut();
        let Some(preload) = state
            .pending_service_worker_navigation_preloads
            .get_mut(&failure.event_id)
        else {
            return;
        };
        if preload.version_id != failure.version_id || preload.run != failure.run {
            return;
        }
        preload.resolver.take()
    }) else {
        return;
    };
    let resolver = v8::Local::new(scope, resolver);
    let reason =
        crate::context_bootstrap::new_dom_exception_value(scope, &failure.message, "NetworkError");
    let _ = resolver.reject(scope, reason);
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    state
        .borrow_mut()
        .pending_service_worker_navigation_preloads
        .remove(&failure.event_id);
}

fn build_service_worker_respond_with_stream_chunk_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_id: ServiceWorkerEventId,
    body_source_id: NetworkBodySourceId,
) -> Option<v8::Local<'s, v8::Function>> {
    let data = ServiceWorkerRespondWithStreamChunkCallbackDataDeclaration {
        event_id: v8::BigInt::new_from_u64(scope, event_id.as_u64()),
        body_source_id: v8::BigInt::new_from_u64(scope, body_source_id),
    }
    .bind(scope)
    .ok()?;
    v8::Function::builder(service_worker_respond_with_stream_chunk_callback)
        .data(data.into())
        .build(scope)
}

fn service_worker_respond_with_stream_chunk_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = v8::Local::<v8::Object>::try_from(args.data()).ok() else {
        return;
    };
    let Some(event_id) = data
        .get(scope, v8str(scope, "eventId").into())
        .and_then(|value| service_worker_event_id_value(scope, value))
        .map(ServiceWorkerEventId::from_u64_for_worker)
    else {
        return;
    };
    let Some(body_source_id) = data
        .get(scope, v8str(scope, "bodySourceId").into())
        .and_then(|value| service_worker_event_id_value(scope, value))
    else {
        return;
    };
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    if !service_worker_stream_chunk_matches_pending_event(&state, event_id, body_source_id) {
        return;
    }
    let Some(bytes) = crate::blob::buffer_source_bytes_from_value(scope, args.get(0)) else {
        throw_type_error(scope, "ReadableStream body chunk bytes are unavailable.");
        return;
    };
    let parent_tx = state.borrow().parent_tx.clone();
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerFetchStreamChunk(
        ServiceWorkerFetchStreamChunk {
            event_id,
            body_source_id,
            bytes,
        },
    ));
}

fn service_worker_stream_chunk_matches_pending_event(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
    body_source_id: NetworkBodySourceId,
) -> bool {
    state
        .borrow()
        .pending_service_worker_fetch_events
        .get(&event_id)
        .is_some_and(|pending| {
            pending.pending_respond_with_stream_body_source_id == Some(body_source_id)
        })
}

fn build_service_worker_respond_with_body_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_id: ServiceWorkerEventId,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Option<v8::Local<'s, v8::Function>> {
    let data = ServiceWorkerRespondWithBodyCallbackDataDeclaration {
        event_id: v8::BigInt::new_from_u64(scope, event_id.as_u64()),
    }
    .bind(scope)
    .ok()?;
    v8::Function::builder(callback)
        .data(data.into())
        .build(scope)
}

fn service_worker_respond_with_body_fulfilled_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(event_id) = service_worker_respond_with_body_callback_event_id(scope, &args) else {
        return;
    };
    let body = materialized_body_bytes_from_value(scope, args.get(0))
        .map_err(|error| format!("FetchEvent.respondWith {error}"));
    service_worker_respond_with_body_settled_for_event(scope, event_id, body);
}

fn service_worker_respond_with_body_rejected_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(event_id) = service_worker_respond_with_body_callback_event_id(scope, &args) else {
        return;
    };
    let reason = js_value_error_string(scope, args.get(0));
    service_worker_respond_with_body_settled_for_event(
        scope,
        event_id,
        Err(format!(
            "FetchEvent.respondWith failed to materialize Response body: {reason}"
        )),
    );
}

fn service_worker_respond_with_body_callback_event_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ServiceWorkerEventId> {
    v8::Local::<v8::Object>::try_from(args.data())
        .ok()
        .and_then(|data| {
            data.get(scope, v8str(scope, "eventId").into())
                .and_then(|value| service_worker_event_id_value(scope, value))
        })
        .map(ServiceWorkerEventId::from_u64_for_worker)
}

fn service_worker_respond_with_body_settled_for_event(
    scope: &mut v8::PinScope<'_, '_>,
    event_id: ServiceWorkerEventId,
    body: Result<Vec<u8>, String>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let handled_settled = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) else {
            return;
        };
        let result = match body {
            Ok(body) => {
                let Some(head) = pending.pending_respond_with_response.take() else {
                    return;
                };
                pending.pending_respond_with_stream_body_source_id = None;
                pending.pending_respond_with_stream_cancel_handle = None;
                ServiceWorkerFetchResult::Response(service_worker_fetch_response_from_materialized(
                    head, body,
                ))
            }
            Err(error) => {
                pending.pending_respond_with_response = None;
                pending.pending_respond_with_stream_body_source_id = None;
                pending.pending_respond_with_stream_cancel_handle = None;
                ServiceWorkerFetchResult::Failure(error)
            }
        };
        pending.pending_respond_with = false;
        pending.completion.result = result;
        settle_service_worker_fetch_handled_for_pending(scope, pending)
    };
    if handled_settled {
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
    maybe_send_service_worker_fetch_completion(&state, event_id, scope);
}

fn service_worker_respond_with_settled_for_event(
    scope: &mut v8::PinScope<'_, '_>,
    event_id: ServiceWorkerEventId,
    result: ServiceWorkerFetchResult,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let handled_settled = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) else {
            return;
        };
        pending.pending_respond_with_response = None;
        pending.pending_respond_with_stream_body_source_id = None;
        pending.pending_respond_with_stream_cancel_handle = None;
        pending.pending_respond_with = false;
        pending.completion.result = result;
        settle_service_worker_fetch_handled_for_pending(scope, pending)
    };
    if handled_settled {
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
    maybe_send_service_worker_fetch_completion(&state, event_id, scope);
}

fn settle_service_worker_fetch_handled_for_pending(
    scope: &mut v8::PinScope<'_, '_>,
    pending: &mut PendingServiceWorkerFetchEvent,
) -> bool {
    let Some(resolver) = pending.handled_resolver.take() else {
        return false;
    };
    let resolver = v8::Local::new(scope, resolver);
    match &pending.completion.result {
        ServiceWorkerFetchResult::Response(_) | ServiceWorkerFetchResult::Fallback => {
            let value = v8::undefined(scope);
            let _ = resolver.resolve(scope, value.into());
        }
        ServiceWorkerFetchResult::Failure(message) => {
            let reason =
                crate::context_bootstrap::new_dom_exception_value(scope, message, "NetworkError");
            let _ = resolver.reject(scope, reason);
        }
    }
    true
}

fn resolve_service_worker_fetch_handled_for_event(
    scope: &mut v8::PinScope<'_, '_>,
    event_id: ServiceWorkerEventId,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let handled_settled = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) else {
            return;
        };
        let Some(resolver) = pending.handled_resolver.take() else {
            return;
        };
        let resolver = v8::Local::new(scope, resolver);
        let value = v8::undefined(scope);
        let _ = resolver.resolve(scope, value.into());
        true
    };
    if handled_settled {
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
}

fn settle_service_worker_fetch_handled_after_dispatch(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let handled_settled = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) else {
            return;
        };
        if !pending.dispatch_finished || pending.pending_respond_with {
            return;
        }
        settle_service_worker_fetch_handled_for_pending(scope, pending)
    };
    if handled_settled {
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
}

fn js_value_error_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> String {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown rejection".to_owned())
}

fn mark_service_worker_lifecycle_dispatch_finished(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let mut state = state.borrow_mut();
    if let Some(pending) = state
        .pending_service_worker_lifecycle_events
        .get_mut(&event_id)
    {
        pending.dispatch_finished = true;
    }
}

fn mark_service_worker_fetch_dispatch_finished(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let mut state = state.borrow_mut();
    if let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) {
        pending.dispatch_finished = true;
    }
}

fn mark_service_worker_fetch_default_prevented(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let mut state = state.borrow_mut();
    if let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id)
        && !pending.respond_with_called
    {
        pending.completion.result = ServiceWorkerFetchResult::Failure(
            "FetchEvent was canceled without respondWith().".to_owned(),
        );
    }
}

fn mark_service_worker_message_dispatch_finished(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let mut state = state.borrow_mut();
    if let Some(pending) = state
        .pending_service_worker_message_events
        .get_mut(&event_id)
    {
        pending.dispatch_finished = true;
    }
}

fn mark_service_worker_notification_dispatch_finished(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let mut state = state.borrow_mut();
    if let Some(pending) = state
        .pending_service_worker_notification_events
        .get_mut(&event_id)
    {
        pending.dispatch_finished = true;
    }
}

fn mark_service_worker_push_dispatch_finished(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let mut state = state.borrow_mut();
    if let Some(pending) = state.pending_service_worker_push_events.get_mut(&event_id) {
        pending.dispatch_finished = true;
    }
}

fn mark_service_worker_sync_dispatch_finished(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let mut state = state.borrow_mut();
    if let Some(pending) = state.pending_service_worker_sync_events.get_mut(&event_id) {
        pending.dispatch_finished = true;
    }
}

fn mark_service_worker_periodic_sync_dispatch_finished(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let mut state = state.borrow_mut();
    if let Some(pending) = state
        .pending_service_worker_periodic_sync_events
        .get_mut(&event_id)
    {
        pending.dispatch_finished = true;
    }
}

fn maybe_send_service_worker_lifecycle_completion(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let (parent_tx, completion) = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_lifecycle_events.get(&event_id) else {
            return;
        };
        if !pending.dispatch_finished || pending.pending_wait_until_count > 0 {
            return;
        }
        let completion = pending.completion.clone();
        state
            .pending_service_worker_lifecycle_events
            .remove(&event_id);
        (state.parent_tx.clone(), completion)
    };
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerLifecycleCompleted(
        completion,
    ));
}

fn maybe_send_service_worker_fetch_completion(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
    scope: &mut v8::PinScope<'_, '_>,
) {
    let (parent_tx, completion, handled_settled) = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_fetch_events.get_mut(&event_id) else {
            return;
        };
        if !pending.dispatch_finished
            || pending.pending_respond_with
            || pending.pending_wait_until_count > 0
        {
            return;
        }
        let handled_settled = settle_service_worker_fetch_handled_for_pending(scope, pending);
        let completion = pending.completion.clone();
        state.pending_service_worker_fetch_events.remove(&event_id);
        (state.parent_tx.clone(), completion, handled_settled)
    };
    if handled_settled {
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
    let _ = parent_tx.send(WorkerToParentMessage::ServiceWorkerFetchCompleted(
        completion,
    ));
}

fn maybe_send_service_worker_message_completion(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let (parent_tx, completion) = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_message_events.get(&event_id) else {
            return;
        };
        if !pending.dispatch_finished || pending.pending_wait_until_count > 0 {
            return;
        }
        let completion = pending.completion.clone();
        let window_interaction_allowed = pending.window_interaction_allowed;
        state
            .pending_service_worker_message_events
            .remove(&event_id);
        if window_interaction_allowed {
            state.service_worker_window_interaction_allowed_count = state
                .service_worker_window_interaction_allowed_count
                .saturating_sub(1);
        }
        (state.parent_tx.clone(), completion)
    };
    let message = WorkerToParentMessage::ServiceWorkerMessageCompleted(completion);
    let _ = parent_tx.send(message);
}

fn maybe_send_service_worker_notification_completion(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let (parent_tx, completion) = {
        let mut state = state.borrow_mut();
        let Some(pending) = state
            .pending_service_worker_notification_events
            .get(&event_id)
        else {
            return;
        };
        if !pending.dispatch_finished || pending.pending_wait_until_count > 0 {
            return;
        }
        let completion = pending.completion.clone();
        let window_interaction_allowed = pending.window_interaction_allowed;
        state
            .pending_service_worker_notification_events
            .remove(&event_id);
        if window_interaction_allowed {
            state.service_worker_window_interaction_allowed_count = state
                .service_worker_window_interaction_allowed_count
                .saturating_sub(1);
        }
        (state.parent_tx.clone(), completion)
    };
    let message = WorkerToParentMessage::ServiceWorkerNotificationCompleted(completion);
    let _ = parent_tx.send(message);
}

fn maybe_send_service_worker_push_completion(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let (parent_tx, completion) = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_push_events.get(&event_id) else {
            return;
        };
        if !pending.dispatch_finished || pending.pending_wait_until_count > 0 {
            return;
        }
        let completion = pending.completion.clone();
        state.pending_service_worker_push_events.remove(&event_id);
        (state.parent_tx.clone(), completion)
    };
    let message = WorkerToParentMessage::ServiceWorkerPushCompleted(completion);
    let _ = parent_tx.send(message);
}

fn maybe_send_service_worker_sync_completion(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let (parent_tx, completion) = {
        let mut state = state.borrow_mut();
        let Some(pending) = state.pending_service_worker_sync_events.get(&event_id) else {
            return;
        };
        if !pending.dispatch_finished || pending.pending_wait_until_count > 0 {
            return;
        }
        let completion = pending.completion.clone();
        state.pending_service_worker_sync_events.remove(&event_id);
        (state.parent_tx.clone(), completion)
    };
    let message = WorkerToParentMessage::ServiceWorkerSyncCompleted(completion);
    let _ = parent_tx.send(message);
}

fn maybe_send_service_worker_periodic_sync_completion(
    state: &Rc<RefCell<super::WorkerGlobalState>>,
    event_id: ServiceWorkerEventId,
) {
    let (parent_tx, completion) = {
        let mut state = state.borrow_mut();
        let Some(pending) = state
            .pending_service_worker_periodic_sync_events
            .get(&event_id)
        else {
            return;
        };
        if !pending.dispatch_finished || pending.pending_wait_until_count > 0 {
            return;
        }
        let completion = pending.completion.clone();
        state
            .pending_service_worker_periodic_sync_events
            .remove(&event_id);
        (state.parent_tx.clone(), completion)
    };
    let message = WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(completion);
    let _ = parent_tx.send(message);
}

pub(super) fn dispatch_worker_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    report: V8ExceptionReport,
    exception: Option<v8::Local<'s, v8::Value>>,
    parent_event_kind: WorkerParentErrorEventKind,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) -> bool {
    dispatch_worker_exception_with_phase(
        scope,
        global,
        report,
        exception,
        parent_event_kind,
        WorkerErrorPhase::Runtime,
        parent_tx,
        script_url,
    )
}

pub(crate) fn dispatch_current_worker_callback_exception(
    scope: &mut v8::PinScope<'_, '_>,
    report: V8ExceptionReport,
) -> bool {
    let Some((parent_tx, script_url)) = worker_exception_report_target(scope) else {
        return false;
    };
    let global = scope.get_current_context().global(scope);
    let exception = report
        .exception
        .as_ref()
        .map(|exception| v8::Local::new(scope, exception));
    dispatch_worker_exception(
        scope,
        global,
        report,
        exception,
        WorkerParentErrorEventKind::ErrorEvent,
        &parent_tx,
        &script_url,
    )
}

pub(super) fn dispatch_worker_exception_with_phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    report: V8ExceptionReport,
    exception: Option<v8::Local<'s, v8::Value>>,
    parent_event_kind: WorkerParentErrorEventKind,
    parent_phase: WorkerErrorPhase,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) -> bool {
    dispatch_worker_exception_with_phase_and_source(
        scope,
        global,
        report,
        exception,
        parent_event_kind,
        parent_phase,
        WorkerErrorSource::Runtime,
        parent_tx,
        script_url,
    )
}

pub(super) fn dispatch_worker_exception_with_phase_and_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    mut report: V8ExceptionReport,
    exception: Option<v8::Local<'s, v8::Value>>,
    parent_event_kind: WorkerParentErrorEventKind,
    parent_phase: WorkerErrorPhase,
    source: WorkerErrorSource,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) -> bool {
    apply_worker_exception_location_overrides(scope, &mut report, exception);
    let handled =
        dispatch_worker_error_event(scope, global, &report, exception, parent_tx, script_url);
    if !handled {
        report_exception_to_parent_with_phase_and_source(
            &report,
            script_url,
            parent_event_kind,
            parent_phase,
            source,
            parent_tx,
        );
    }
    handled
}

fn dispatch_worker_global_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let event = new_worker_message_event(scope, event_type, data, None, ports);
    set_event_dispatch_fields(scope, global, event);

    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        event_type,
    );
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event,
            "WorkerGlobalScope message listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                event_type,
                listener.original,
                listener.capture,
            );
        }
    }

    clear_event_dispatch_fields(scope, event);
}

/// Dispatch a parent `postMessage()` delivery into the worker global.
pub(super) fn dispatch_message_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    payload: &V8StructuredClonePayload,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);

    {
        let try_catch = pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        let global = ctx.global(&scope);

        if let Some((data, ports)) =
            structured_deserialize_value_for_message_event(&mut scope, payload)
        {
            dispatch_worker_global_message_event(
                &mut scope, global, "message", data, ports, parent_tx, script_url,
            );
        } else {
            // `postMessage` delivery failures are reported to the receiver as a
            // `messageerror` event. This matters for secure-context-only host
            // objects such as CryptoKey: an insecure worker cannot deserialize
            // the object, but the failure must not be hidden as a normal
            // message with `undefined` data.
            if scope.has_caught() {
                scope.reset();
            }
            let data = v8::null(&scope).into();
            let ports = v8::Array::new(&scope, 0);
            dispatch_worker_global_message_event(
                &mut scope,
                global,
                "messageerror",
                data,
                ports,
                parent_tx,
                script_url,
            );
        }
    }

    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_shared_worker_connect_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    port_id: MessagePortId,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let global = ctx.global(scope);

    let mut source = None;
    let ports = if let Some(port) = ensure_message_port_wrapper_for_id(scope, port_id) {
        register_shared_worker_connection_port(scope, port_id);
        source = Some(port.into());
        serialize_v8_array(scope, [port]).unwrap_or_else(|| v8::Array::new(scope, 1))
    } else {
        v8::Array::new(scope, 1)
    };
    let _ = ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    let data = v8str(scope, "").into();
    let event = new_worker_message_event(scope, "connect", data, source, ports);
    set_event_dispatch_fields(scope, global, event);

    let listeners = simple_object_event_listeners_snapshot(
        scope,
        global,
        WORKER_GLOBAL_LISTENERS_SLOT,
        "connect",
    );
    for listener in &listeners {
        if let Err(error) = invoke_worker_listener(
            scope,
            listener,
            global,
            event,
            "SharedWorkerGlobalScope connect listener",
        ) {
            let (report, exception) = *error;
            let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
            dispatch_worker_exception(
                scope,
                global,
                report,
                exception,
                WorkerParentErrorEventKind::ErrorEvent,
                parent_tx,
                script_url,
            );
        }
        if listener.once {
            simple_object_event_remove_listener_value_for_type(
                scope,
                global,
                WORKER_GLOBAL_LISTENERS_SLOT,
                "connect",
                listener.original,
                listener.capture,
            );
        }
    }

    clear_event_dispatch_fields(scope, event);
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

pub(super) fn dispatch_message_port_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    port_id: MessagePortId,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) -> bool {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    if crate::worker::worker_termination_requested(scope) {
        return true;
    }
    let mut callback_errors = Vec::new();
    let callback_dispatched = dispatch_message_port_events_for_port_collecting_errors(
        scope,
        port_id,
        Some(&mut callback_errors),
    );
    if scope.is_execution_terminating() || crate::worker::worker_termination_requested(scope) {
        return true;
    }
    if callback_dispatched {
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
    if scope.is_execution_terminating() || crate::worker::worker_termination_requested(scope) {
        return true;
    }
    if callback_errors.is_empty() {
        return false;
    }

    let global = ctx.global(scope);
    for mut report in callback_errors {
        let exception_global = report.exception.take();
        let exception = exception_global
            .as_ref()
            .map(|exception| v8::Local::new(scope, exception));
        dispatch_worker_exception(
            scope,
            global,
            report,
            exception,
            WorkerParentErrorEventKind::ErrorEvent,
            parent_tx,
            script_url,
        );
        if scope.is_execution_terminating() || crate::worker::worker_termination_requested(scope) {
            return true;
        }
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    scope.is_execution_terminating() || crate::worker::worker_termination_requested(scope)
}

pub(super) fn dispatch_broadcast_channel_event(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    channel_id: BroadcastChannelId,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    if crate::context_bootstrap::dispatch_broadcast_channel_events_for_channel(scope, channel_id) {
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
}

/// Fire a timer callback.
pub(super) fn fire_timer_callback(
    isolate: &mut v8::OwnedIsolate,
    timer: &ActiveTimer,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
    script_url: &str,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = timer.callback.target_context(scope);
    let scope = &mut v8::ContextScope::new(scope, ctx);

    let global = ctx.global(scope);

    let callback_error = match timer.callback.invoke(scope, &timer.extra_args) {
        WorkerTimerCallbackOutcome::Returned => None,
        WorkerTimerCallbackOutcome::Threw(report) => {
            trace!(
                timer_id = timer.id,
                message = report.summary.as_str(),
                "timer callback error"
            );
            Some(report)
        }
    };
    if let Some(report) = callback_error {
        let exception = report
            .exception
            .as_ref()
            .map(|value| v8::Local::new(scope, value));
        dispatch_worker_exception(
            scope,
            global,
            *report,
            exception,
            WorkerParentErrorEventKind::ErrorEvent,
            parent_tx,
            script_url,
        );
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

/// Create a V8 ScriptOrigin for error reporting.
pub(super) fn create_script_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    url: &str,
) -> v8::ScriptOrigin<'s> {
    create_script_origin_with_kind(scope, url, false)
}

pub(super) fn create_script_origin_with_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    url: &str,
    is_module: bool,
) -> v8::ScriptOrigin<'s> {
    let name = v8::String::new(scope, url).unwrap();
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
        is_module,
        None,
    )
}
