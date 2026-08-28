use super::*;
use crate::callback_invocation::{CallbackInvocation, CallbackInvocationOutcome, CallbackInvoker};
use crate::context_bootstrap::events::{
    clear_event_dispatch_fields, event_internal_bool_flag, mark_event_trusted,
    set_event_dispatch_fields, set_event_internal_flag,
};
use crate::context_bootstrap::{EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT};
use crate::exception_reporting::CallbackExceptionLogLevel;
use crate::types::MessagePortId;
use crate::worker::worker_message_port_wrapper;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct MessagePortMessageEventInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    data: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    source: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    ports: v8::Local<'scope, v8::Array>,
}

enum MessagePortEventCallback<'s> {
    Handler {
        order: f64,
        label: &'static str,
        callback: v8::Local<'s, v8::Function>,
    },
    Listener {
        order: f64,
        id: MessagePortEventListenerId,
    },
}

impl MessagePortEventCallback<'_> {
    fn order(&self) -> f64 {
        match self {
            Self::Handler { order, .. } | Self::Listener { order, .. } => *order,
        }
    }
}

enum MessagePortDispatchTarget<'s> {
    Window {
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
        realm_token: crate::native_bridge::RuntimeObservableContextToken,
        context: v8::Local<'s, v8::Context>,
        wrapper: v8::Local<'s, v8::Object>,
    },
    Worker(v8::Local<'s, v8::Object>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MessagePortDeliveryRunResult {
    Idle,
    Consumed { callback_dispatched: bool },
}

pub(in crate::context_bootstrap) fn schedule_message_port_delivery(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
) {
    if let Some(registry) = current_message_port_registry(scope) {
        registry.wake_message_port_if_pending(port_id);
    }
}

pub(crate) fn dispatch_message_port_events_for_port_collecting_errors(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    mut callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) -> bool {
    let Some(target) = message_port_dispatch_target(scope, port_id) else {
        return false;
    };
    match target {
        MessagePortDispatchTarget::Window {
            dispatch_scope,
            realm_token,
            context,
            wrapper,
        } => {
            let scope = &mut v8::ContextScope::new(scope, context);
            if crate::native_bridge::current_runtime_observable_context_token(scope)
                != Some(realm_token)
            {
                if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
                    unsafe { &mut *host_ptr }.retire_message_port(port_id);
                }
                tracing::debug!(
                    port_id,
                    ?realm_token,
                    "closed MessagePort for mismatched execution context"
                );
                return false;
            }
            let previous_owner_context = dispatch_scope.enter(scope);
            let dispatched = dispatch_message_port_events_in_current_context(
                scope,
                port_id,
                wrapper,
                callback_errors.as_deref_mut(),
            );
            dispatch_scope.restore(scope, previous_owner_context);
            dispatched
        }
        MessagePortDispatchTarget::Worker(wrapper) => {
            dispatch_message_port_events_in_current_context(
                scope,
                port_id,
                wrapper,
                callback_errors,
            )
        }
    }
}

/// Consume at most one MessagePort event after the Page arbiter has matched the
/// selected task against the port's current exact Window attachment.
pub(crate) fn dispatch_one_authorized_message_port_event(
    scope: &mut v8::PinScope<'_, '_>,
    port_id: MessagePortId,
    expected: crate::native_bridge::WindowExecutionContextIdentity,
    same_attachment_task_is_ready: bool,
) -> MessagePortDeliveryRunResult {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return MessagePortDeliveryRunResult::Idle;
    };
    let Some((dispatch_scope, realm_token, context, wrapper)) =
        (unsafe { &*host_ptr }).authorized_message_port_dispatch_target(scope, port_id, expected)
    else {
        return MessagePortDeliveryRunResult::Idle;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    assert_eq!(
        crate::native_bridge::current_runtime_observable_context_token(scope),
        Some(realm_token),
        "authorized MessagePort binding diverged from its V8 context token"
    );
    let previous_owner_context = dispatch_scope.enter(scope);
    let result = dispatch_one_message_port_event_in_current_context(scope, port_id, wrapper, None);
    dispatch_scope.restore(scope, previous_owner_context);
    if !same_attachment_task_is_ready
        && matches!(result, MessagePortDeliveryRunResult::Consumed { .. })
        && let Some(registry) = current_message_port_registry(scope)
    {
        // One selected Page turn consumes one event. If the port still has a
        // backlog, publish a fresh task through whichever owner is attached
        // after the callback (the handler may have transferred the port).
        registry.wake_message_port_if_pending(port_id);
    }
    if message_port_is_closed(scope, wrapper)
        && current_message_port_registry(scope)
            .is_none_or(|registry| !registry.contains_message_port(port_id))
    {
        // `close()` during a callback detaches the JS-facing endpoint but
        // accepted delivery tasks still target this exact wrapper. Release the
        // retained binding only after the registry has consumed that backlog.
        forget_message_port_wrapper(scope, port_id);
    }
    result
}

fn message_port_dispatch_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
) -> Option<MessagePortDispatchTarget<'s>> {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return unsafe { &mut *host_ptr }
            .message_port_dispatch_target(scope, port_id)
            .map(|(dispatch_scope, realm_token, context, wrapper)| {
                MessagePortDispatchTarget::Window {
                    dispatch_scope,
                    realm_token,
                    context,
                    wrapper,
                }
            });
    }
    worker_message_port_wrapper(scope, port_id).map(MessagePortDispatchTarget::Worker)
}

fn dispatch_message_port_events_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
    target: v8::Local<'s, v8::Object>,
    mut callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) -> bool {
    let mut dispatched = false;
    loop {
        // Worker termination can interrupt a callback while this wake is
        // draining a backlog. No later message from the same task may enter
        // the terminating isolate.
        if scope.is_execution_terminating() || crate::worker::worker_termination_requested(scope) {
            return dispatched;
        }
        match dispatch_one_message_port_event_in_current_context(
            scope,
            port_id,
            target,
            callback_errors.as_deref_mut(),
        ) {
            MessagePortDeliveryRunResult::Idle => return dispatched,
            MessagePortDeliveryRunResult::Consumed {
                callback_dispatched,
            } => dispatched |= callback_dispatched,
        }
    }
}

fn dispatch_one_message_port_event_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
    target: v8::Local<'s, v8::Object>,
    callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) -> MessagePortDeliveryRunResult {
    let Some(registry) = current_message_port_registry(scope) else {
        return MessagePortDeliveryRunResult::Idle;
    };
    let onmessage = message_port_onmessage(scope, target);
    let started = message_port_is_started(scope, target);
    if onmessage.is_none() && !started {
        if !registry.take_pending_message_port_close(port_id) {
            return MessagePortDeliveryRunResult::Idle;
        }
        return MessagePortDeliveryRunResult::Consumed {
            callback_dispatched: dispatch_message_port_close_event(
                scope,
                port_id,
                target,
                callback_errors,
            ),
        };
    }
    let Some(payload) = registry.take_pending_message_port_message(port_id) else {
        if !registry.take_pending_message_port_close(port_id) {
            return MessagePortDeliveryRunResult::Idle;
        }
        return MessagePortDeliveryRunResult::Consumed {
            callback_dispatched: dispatch_message_port_close_event(
                scope,
                port_id,
                target,
                callback_errors,
            ),
        };
    };

    let target_origin = crate::context_bootstrap::current_runtime_message_origin(scope);
    let target_agent_cluster =
        crate::context_bootstrap::current_runtime_message_agent_cluster(scope);
    let callback_dispatched = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        if !crate::context_bootstrap::wasm_module_message_allowed_for_target(
            &payload,
            target_origin.as_deref(),
            target_agent_cluster,
        ) {
            dispatch_message_port_messageerror_event(
                &mut scope,
                &registry,
                port_id,
                target,
                callback_errors,
            )
        } else if let Some((data, ports)) =
            crate::context_bootstrap::structured_deserialize_value_for_message_event(
                &mut scope, &payload,
            )
        {
            if crate::worker::worker_termination_requested(&mut scope) {
                false
            } else {
                dispatch_message_port_message_event(
                    &mut scope,
                    &registry,
                    port_id,
                    target,
                    "message",
                    data,
                    ports,
                    callback_errors,
                )
            }
        } else if scope.has_terminated()
            || scope.is_execution_terminating()
            || crate::worker::worker_termination_requested(&mut scope)
        {
            // `Worker.terminate()` interrupts V8 immediately. If that races a
            // queued MessagePort delivery, a valid payload can return an empty
            // MaybeLocal solely because the worker realm is terminating. This
            // is teardown, not a structured-clone failure, so it must not
            // become a receiver-visible `messageerror`.
            false
        } else {
            if scope.has_caught() {
                scope.reset();
            }
            dispatch_message_port_messageerror_event(
                &mut scope,
                &registry,
                port_id,
                target,
                callback_errors,
            )
        }
    };
    MessagePortDeliveryRunResult::Consumed {
        callback_dispatched,
    }
}

fn dispatch_message_port_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &crate::message_port_runtime::SharedMessagePortRegistry,
    port_id: MessagePortId,
    target: v8::Local<'s, v8::Object>,
    event_type: &'static str,
    data: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
    callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) -> bool {
    let Some(event) = new_message_event(scope, event_type, data, v8::null(scope).into(), ports)
    else {
        return false;
    };
    dispatch_message_port_event(
        scope,
        registry,
        port_id,
        target,
        event_type,
        event,
        callback_errors,
    )
}

fn dispatch_message_port_messageerror_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &crate::message_port_runtime::SharedMessagePortRegistry,
    port_id: MessagePortId,
    target: v8::Local<'s, v8::Object>,
    callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) -> bool {
    let data = v8::null(scope).into();
    let ports = v8::Array::new(scope, 0);
    dispatch_message_port_message_event(
        scope,
        registry,
        port_id,
        target,
        "messageerror",
        data,
        ports,
        callback_errors,
    )
}

fn dispatch_message_port_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &crate::message_port_runtime::SharedMessagePortRegistry,
    port_id: MessagePortId,
    target: v8::Local<'s, v8::Object>,
    event_type: &'static str,
    event: v8::Local<'s, v8::Object>,
    mut callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) -> bool {
    mark_event_trusted(scope, event);
    set_event_dispatch_fields(scope, target, event);

    // MessagePort delivery is an async host callback surface. Keep the same
    // local TryCatch contract as other event dispatch paths.
    let mut callbacks = message_port_event_callbacks(scope, port_id, target, event_type);
    callbacks.sort_by(|left, right| left.order().total_cmp(&right.order()));

    registry.begin_message_port_message_delivery(port_id);
    let mut dispatched = false;
    for callback in callbacks {
        match callback {
            MessagePortEventCallback::Handler {
                label, callback, ..
            } => {
                if let Err(report) = invoke_callback_with_report(
                    scope,
                    "callback",
                    "host callback threw",
                    CallbackExceptionLogLevel::Debug,
                    label,
                    callback,
                    target.into(),
                    &[event.into()],
                ) {
                    project_legacy_message_port_handler_exception(
                        scope,
                        *report,
                        callback_errors.as_deref_mut(),
                    );
                }
                dispatched = true;
            }
            MessagePortEventCallback::Listener { id, .. } => {
                let Some(listener) = claim_message_port_event_listener(scope, port_id, id) else {
                    continue;
                };
                set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, listener.passive);
                dispatched |= invoke_message_port_event_listener(
                    scope,
                    event_type,
                    listener.callback,
                    target,
                    event,
                    callback_errors.as_deref_mut(),
                );
                set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);
            }
        }
        if event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT) {
            break;
        }
    }
    registry.finish_message_port_message_delivery(port_id);
    clear_event_dispatch_fields(scope, event);
    dispatched
}

fn dispatch_message_port_close_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
    target: v8::Local<'s, v8::Object>,
    mut callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) -> bool {
    let Some(event) = new_simple_event(scope, "close") else {
        return false;
    };
    mark_event_trusted(scope, event);
    set_event_dispatch_fields(scope, target, event);
    let mut callbacks = message_port_event_callbacks(scope, port_id, target, "close");
    callbacks.sort_by(|left, right| left.order().total_cmp(&right.order()));
    let mut dispatched = false;
    for callback in callbacks {
        match callback {
            MessagePortEventCallback::Handler {
                label, callback, ..
            } => {
                if let Err(report) = invoke_callback_with_report(
                    scope,
                    "callback",
                    "host callback threw",
                    CallbackExceptionLogLevel::Debug,
                    label,
                    callback,
                    target.into(),
                    &[event.into()],
                ) {
                    project_legacy_message_port_handler_exception(
                        scope,
                        *report,
                        callback_errors.as_deref_mut(),
                    );
                }
                dispatched = true;
            }
            MessagePortEventCallback::Listener { id, .. } => {
                let Some(listener) = claim_message_port_event_listener(scope, port_id, id) else {
                    continue;
                };
                set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, listener.passive);
                dispatched |= invoke_message_port_event_listener(
                    scope,
                    "close",
                    listener.callback,
                    target,
                    event,
                    callback_errors.as_deref_mut(),
                );
                set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);
            }
        }
        if event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT) {
            break;
        }
    }
    clear_event_dispatch_fields(scope, event);
    dispatched
}

fn message_port_event_callbacks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    port_id: MessagePortId,
    target: v8::Local<'s, v8::Object>,
    event_type: &'static str,
) -> Vec<MessagePortEventCallback<'s>> {
    let mut callbacks = Vec::new();
    match event_type {
        "message" => {
            if let Some(onmessage) = message_port_onmessage(scope, target) {
                callbacks.push(MessagePortEventCallback::Handler {
                    order: message_port_onmessage_order(scope, target).unwrap_or(-1.0),
                    label: "MessagePort onmessage",
                    callback: onmessage,
                });
            }
        }
        "messageerror" => {
            if let Some(onmessageerror) = message_port_onmessageerror(scope, target) {
                callbacks.push(MessagePortEventCallback::Handler {
                    order: message_port_onmessageerror_order(scope, target).unwrap_or(-1.0),
                    label: "MessagePort onmessageerror",
                    callback: onmessageerror,
                });
            }
        }
        "close" => {
            if let Some(onclose) = message_port_onclose(scope, target) {
                callbacks.push(MessagePortEventCallback::Handler {
                    order: message_port_onclose_order(scope, target).unwrap_or(-1.0),
                    label: "MessagePort onclose",
                    callback: onclose,
                });
            }
        }
        _ => {}
    }

    if event_type != "message" || message_port_is_started(scope, target) {
        callbacks.extend(
            message_port_event_listener_snapshots(scope, port_id, event_type)
                .into_iter()
                .map(|snapshot| MessagePortEventCallback::Listener {
                    order: snapshot.order,
                    id: snapshot.id,
                }),
        );
    }
    callbacks
}

fn invoke_message_port_event_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    callback: PreparedMessagePortEventListenerCallback,
    target: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
    callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) -> bool {
    let callback_name = match event_type {
        "message" => "MessagePort message listener",
        "messageerror" => "MessagePort messageerror listener",
        "close" => "MessagePort close listener",
        _ => "MessagePort listener",
    };
    let arguments = [event.into()];
    let (invocation, relevant_identity) = match callback {
        PreparedMessagePortEventListenerCallback::Window(callback) => {
            let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
                return false;
            };
            let relevant_identity = callback.relevant_identity();
            (
                CallbackInvocation::new(
                    callback.callback(scope),
                    target.into(),
                    callback.relevant_context(scope),
                    callback.incumbent_context(scope),
                    callback.is_callable(),
                    "handleEvent",
                    &arguments,
                    Some(event),
                )
                .with_execution_context_currentness(host_ptr, relevant_identity),
                relevant_identity,
            )
        }
        PreparedMessagePortEventListenerCallback::Worker(callback) => (
            CallbackInvocation::new(
                callback.callback(scope),
                target.into(),
                callback.relevant_context(scope),
                callback.incumbent_context(scope),
                callback.callable_at_conversion(),
                "handleEvent",
                &arguments,
                None,
            ),
            None,
        ),
    };

    match CallbackInvoker::invoke(
        scope,
        "event listener",
        "MessagePort event listener threw",
        CallbackExceptionLogLevel::Debug,
        callback_name,
        invocation,
    ) {
        CallbackInvocationOutcome::Returned(_) => true,
        CallbackInvocationOutcome::Threw(report) => {
            project_message_port_callback_exception(
                scope,
                event_type,
                relevant_identity,
                *report,
                callback_errors,
            );
            true
        }
        CallbackInvocationOutcome::Retired => false,
    }
}

fn project_message_port_callback_exception(
    scope: &mut v8::PinScope<'_, '_>,
    event_type: &str,
    relevant_identity: Option<crate::native_bridge::WindowExecutionContextIdentity>,
    report: V8ExceptionReport,
    callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) {
    if let Some(callback_errors) = callback_errors {
        callback_errors.push(report);
    } else if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        crate::host::report_event_callback_exception(
            scope,
            host_ptr,
            event_type,
            relevant_identity,
            None,
            &report,
        );
    } else {
        let _ = crate::worker::dispatch_current_worker_callback_exception(scope, report);
    }
}

fn project_legacy_message_port_handler_exception(
    scope: &mut v8::PinScope<'_, '_>,
    report: V8ExceptionReport,
    callback_errors: Option<&mut Vec<V8ExceptionReport>>,
) {
    if let Some(callback_errors) = callback_errors {
        callback_errors.push(report);
    } else {
        let _ = crate::worker::dispatch_current_worker_callback_exception(scope, report);
    }
}

fn new_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    source: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let message_ctor = global
        .get(scope, v8str(scope, "MessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let init = MessagePortMessageEventInitDeclaration::new(data, source, ports)
        .bind(scope)
        .expect("MessagePort MessageEvent init declaration should bind");
    let event_type = v8_string(scope, event_type)?;
    message_ctor.new_instance(scope, &[event_type.into(), init.into()])
}

fn new_simple_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let event_ctor = global
        .get(scope, v8str(scope, "Event").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let event_type = v8_string(scope, event_type)?;
    event_ctor.new_instance(scope, &[event_type.into()])
}
