use crate::web_api_interfaces;
use std::pin::pin;

use moli_webapi_declare::WebApiObject;

use crate::context_bootstrap::{
    dispatch_simple_event_target_event, events::construct_original_event,
    simple_object_event_listeners_snapshot,
};
use crate::structured_clone::V8StructuredClonePayload;
use crate::util::v8str;
use crate::worker::{WorkerParentErrorEventKind, WorkerToParentMessage};

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerHostErrorEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    message: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    filename: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    cancelable: bool,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ErrorEvent, prototype = "Object", scope_lifetime = 'scope)]
struct WorkerHostErrorEventFallbackDeclaration<'scope, 'text> {
    #[webapi(data_property, enumerable)]
    r#type: &'static str,
    #[webapi(data_property, enumerable)]
    message: &'text str,
    #[webapi(data_property, enumerable)]
    filename: &'text str,
    #[webapi(data_property, enumerable)]
    lineno: u32,
    #[webapi(data_property, enumerable)]
    colno: u32,
    #[webapi(data_property, enumerable)]
    error: v8::Local<'scope, v8::Value>,
}

pub(crate) fn dispatch_worker_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    message: &WorkerToParentMessage,
) -> bool {
    match message {
        WorkerToParentMessage::Post(payload) => {
            dispatch_onmessage(scope, worker, payload);
            true
        }
        WorkerToParentMessage::Error {
            message,
            filename,
            lineno,
            colno,
            event_kind,
            ..
        } => {
            let error = v8::null(scope).into();
            dispatch_worker_error_event_with_kind(
                scope,
                worker,
                message,
                filename,
                *lineno,
                *colno,
                error,
                *event_kind,
            )
        }
        WorkerToParentMessage::SubresourceNetwork(_)
        | WorkerToParentMessage::PendingSubresourceFetch(_)
        | WorkerToParentMessage::PendingSubresourceFetchCanceled { .. }
        | WorkerToParentMessage::SubresourceContinue(_)
        | WorkerToParentMessage::WebSocketSubresource(_)
        | WorkerToParentMessage::WebSocketLifecycle(_)
        | WorkerToParentMessage::WebSocketFrame(_)
        | WorkerToParentMessage::Console(_)
        | WorkerToParentMessage::RuntimeInspectorMessages(_)
        | WorkerToParentMessage::ServiceWorkerLifecycleCompleted(_)
        | WorkerToParentMessage::ServiceWorkerFetchCompleted(_)
        | WorkerToParentMessage::ServiceWorkerFetchStreamStarted(_)
        | WorkerToParentMessage::ServiceWorkerFetchStreamChunk(_)
        | WorkerToParentMessage::ServiceWorkerMessageCompleted(_)
        | WorkerToParentMessage::ServiceWorkerNotificationCompleted(_)
        | WorkerToParentMessage::ServiceWorkerPushCompleted(_)
        | WorkerToParentMessage::ServiceWorkerPushSubscribe(_)
        | WorkerToParentMessage::ServiceWorkerPushGetSubscription(_)
        | WorkerToParentMessage::ServiceWorkerPushUnsubscribe(_)
        | WorkerToParentMessage::ServiceWorkerSyncCompleted(_)
        | WorkerToParentMessage::ServiceWorkerPeriodicSyncCompleted(_)
        | WorkerToParentMessage::ServiceWorkerShowNotification(_)
        | WorkerToParentMessage::ServiceWorkerGetNotifications(_)
        | WorkerToParentMessage::ServiceWorkerSyncRegistration(_)
        | WorkerToParentMessage::ServiceWorkerSyncGetTags(_)
        | WorkerToParentMessage::ServiceWorkerPeriodicSyncRegistration(_)
        | WorkerToParentMessage::ServiceWorkerPeriodicSyncGetTags(_)
        | WorkerToParentMessage::ServiceWorkerPeriodicSyncUnregistration(_)
        | WorkerToParentMessage::ServiceWorkerCloseNotification(_)
        | WorkerToParentMessage::ServiceWorkerClientMessage(_)
        | WorkerToParentMessage::ServiceWorkerWorkerMessage(_)
        | WorkerToParentMessage::ServiceWorkerClientQuery(_)
        | WorkerToParentMessage::ServiceWorkerClientNavigate(_)
        | WorkerToParentMessage::ServiceWorkerClientFocus(_)
        | WorkerToParentMessage::ServiceWorkerClientsOpenWindow(_)
        | WorkerToParentMessage::ServiceWorkerSkipWaiting { .. }
        | WorkerToParentMessage::ServiceWorkerClientsClaim { .. }
        | WorkerToParentMessage::ServiceWorkerImportedScriptLoaded { .. }
        | WorkerToParentMessage::SharedWorkerClosed
        | WorkerToParentMessage::RuntimeInspectorResponse(_) => false,
    }
}

#[cfg(test)]
pub(crate) fn dispatch_worker_messages<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(handle_ptr) = super::constructor::get_worker_handle(scope, worker) else {
        return false;
    };
    let handle = unsafe { &mut *handle_ptr };
    let mut dispatched = false;
    while let Ok(message) = handle.try_recv() {
        if dispatch_worker_event(scope, worker, &message) {
            dispatched = true;
        }
    }
    dispatched
}

fn dispatch_onmessage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    payload: &V8StructuredClonePayload,
) {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    if let Some((data, ports)) =
        crate::context_bootstrap::structured_deserialize_value_for_message_event(
            &mut scope, payload,
        )
    {
        dispatch_worker_message_event(&mut scope, worker, "message", data, ports);
        return;
    }

    // Structured clone decoding can fail at the receiver when the target
    // context does not expose a host object type, for example CryptoKey in a
    // non-secure context. Web Messaging reports that delivery failure as
    // `messageerror`; dispatching a normal `message` with `undefined` would
    // make callers observe a successful but corrupted delivery.
    if scope.has_caught() {
        scope.reset();
    }
    let data = v8::null(&scope).into();
    let ports = v8::Array::new(&scope, 0);
    dispatch_worker_message_event(&mut scope, worker, "messageerror", data, ports);
}

fn dispatch_worker_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
) {
    let event = new_message_event(scope, event_type, data, ports);
    let _ = dispatch_simple_event_target_event(scope, worker, event_type, event);
}

pub(crate) fn worker_has_message_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) -> bool {
    !simple_object_event_listeners_snapshot(scope, worker, "message").is_empty()
}

pub(crate) fn worker_has_message_delivery_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) -> bool {
    worker_has_message_listener(scope, worker)
        || !simple_object_event_listeners_snapshot(scope, worker, "messageerror").is_empty()
}

pub(crate) fn flush_pending_worker_messages_for_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
) {
    let _ = (scope, worker);
}

pub(crate) fn dispatch_worker_error_event_with_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    worker: v8::Local<'s, v8::Object>,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
    event_kind: WorkerParentErrorEventKind,
) -> bool {
    let event = match event_kind {
        // Fetch/parse failures fire a plain Event with the default flags.
        WorkerParentErrorEventKind::Event => construct_original_event(scope, "error"),
        WorkerParentErrorEventKind::ErrorEvent => Some(new_error_event(
            scope, message, filename, lineno, colno, error,
        )),
    };
    let Some(event) = event else { return false };
    crate::context_bootstrap::mark_event_trusted(scope, event);
    crate::context_bootstrap::dispatch_simple_event_target_event_with_checkpoint(
        scope,
        worker,
        "error",
        event,
        worker_error_microtask_checkpoint,
    )
    .uncanceled
}

fn worker_error_microtask_checkpoint(scope: &mut v8::PinScope<'_, '_>) {
    // Blink observes cancellation after microtasks queued by error listeners.
    scope.perform_microtask_checkpoint();
    crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(scope);
}

fn new_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Object> {
    let source = v8::null(scope).into();
    crate::context_bootstrap::construct_original_message_event(
        scope,
        event_type,
        data,
        "",
        &[],
        source,
        ports,
    )
    .expect("worker host MessageEvent should initialize")
}

fn new_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(error_ctor) = global
        .get(scope, v8str(scope, "ErrorEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let init = WorkerHostErrorEventInitDeclaration::new(
            v8::String::new(scope, message).unwrap(),
            v8::String::new(scope, filename).unwrap(),
            lineno,
            colno,
            true,
            error,
        )
        .bind(scope)
        .expect("worker host ErrorEvent init declaration should bind");
        if let Some(event) = error_ctor.new_instance(
            scope,
            &[v8::String::new(scope, "error").unwrap().into(), init.into()],
        ) {
            return event;
        }
    }

    WorkerHostErrorEventFallbackDeclaration::new("error", message, filename, lineno, colno, error)
        .bind(scope)
        .expect("worker host ErrorEvent fallback declaration should bind")
}
