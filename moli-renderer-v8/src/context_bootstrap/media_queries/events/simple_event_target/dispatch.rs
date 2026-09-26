use super::*;
use crate::{
    callback_invocation::{CallbackInvocation, CallbackInvocationOutcome, CallbackInvoker},
    context_bootstrap::events::{
        EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, error_event_handler_arguments,
        event_internal_bool_flag, set_event_internal_flag,
    },
    context_bootstrap::{EventHandlerType, apply_event_handler_return_value},
    exception_reporting::CallbackExceptionLogLevel,
    host::report_event_callback_exception,
    util::context_host_ptr_from_global_bridge,
    web_api_interfaces,
};

fn event_stop_immediate_propagation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    event_internal_bool_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT)
}

pub(in crate::context_bootstrap::media_queries::events::simple_event_target) fn simple_object_event_target_dispatch<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    slot_name: &str,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
) {
    let Some((event, event_type)) =
        crate::context_bootstrap::event_target_dispatch::prepare_script_dispatch(
            scope,
            args.get(0),
        )
    else {
        rv.set_bool(false);
        return;
    };

    let target = args.this();
    let dispatch_result =
        dispatch_simple_event_target_event(scope, target, slot_name, &event_type, event);
    rv.set(v8::Boolean::new(scope, dispatch_result).into());
}

pub(crate) fn dispatch_simple_event_target_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    dispatch_simple_event_target_event_collecting_errors(
        scope, target, slot_name, event_type, event, None,
    )
    .uncanceled
}

pub(crate) struct SimpleEventDispatchResult {
    pub(crate) uncanceled: bool,
    pub(crate) dispatched: bool,
}

pub(crate) fn dispatch_simple_event_target_event_collecting_errors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
    callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
) -> SimpleEventDispatchResult {
    dispatch_simple_event_target_event_with_original_target_collecting_errors(
        scope,
        target,
        target,
        slot_name,
        event_type,
        event,
        callback_errors,
    )
}

pub(crate) fn dispatch_simple_event_target_event_with_original_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    original_target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
) -> bool {
    dispatch_simple_event_target_event_with_original_target_collecting_errors(
        scope,
        target,
        original_target,
        slot_name,
        event_type,
        event,
        None,
    )
    .uncanceled
}

fn dispatch_simple_event_target_event_with_original_target_collecting_errors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    original_target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
    mut callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
) -> SimpleEventDispatchResult {
    let mut dispatched = false;
    let can_invoke =
        crate::context_bootstrap::event_target_dispatch::begin_dispatch_with_original_target(
            scope,
            target,
            original_target,
            event,
        );

    let error_arguments = if event_type == "error"
        && (web_api_interfaces::WorkerGlobalScope::is_instance(scope, target)
            || web_api_interfaces::Window::is_instance(scope, target))
    {
        error_event_handler_arguments(scope, event)
    } else {
        None
    };
    let ordinary_arguments = [event.into()];
    let handler_arguments = error_arguments
        .as_ref()
        .map_or(ordinary_arguments.as_slice(), |values| values.as_slice());
    let handler_type = if error_arguments.is_some() {
        EventHandlerType::OnErrorEventHandler
    } else {
        EventHandlerType::EventHandler
    };

    if can_invoke && !simple_event_target_uses_ordered_handlers(scope, target) {
        let handler_name = format!("on{event_type}");
        if let Some(handler_key) = v8_string(scope, &handler_name)
            && let Some(handler_value) = target.get(scope, handler_key.into())
            && let Ok(handler) = v8::Local::<v8::Object>::try_from(handler_value)
            && handler.is_callable()
        {
            let current_context = scope.get_current_context();
            let relevant_context = handler
                .get_creation_context(scope)
                .unwrap_or(current_context);
            let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);
            let outcome = invoke_simple_event_callback(
                scope,
                event_type,
                &format!("simple event target {handler_name}"),
                handler,
                relevant_context,
                incumbent_context,
                true,
                target.into(),
                handler_arguments,
                event,
                callback_errors.as_deref_mut(),
            );
            dispatched |= outcome.invoked;
            if let Some(returned) = outcome.value {
                apply_event_handler_return_value(
                    scope,
                    event,
                    v8::Local::new(scope, &returned),
                    handler_type,
                );
            }
        }
    }

    if !event_internal_bool_flag(
        scope,
        event,
        crate::context_bootstrap::EVENT_STOP_PROPAGATION_SLOT,
    ) {
        'phases: for capture_phase in [true, false] {
            // DOM clones the listener list for each invocation phase. A listener
            // added during capture can participate in the subsequent bubble phase.
            let result = invoke_simple_event_target_listeners(
                scope,
                target,
                slot_name,
                event_type,
                event,
                capture_phase,
                handler_arguments,
                handler_type,
                callback_errors.as_deref_mut(),
            );
            dispatched |= result.dispatched;
            if event_stop_immediate_propagation(scope, event) {
                break 'phases;
            }
            if event_internal_bool_flag(
                scope,
                event,
                crate::context_bootstrap::EVENT_STOP_PROPAGATION_SLOT,
            ) {
                break;
            }
        }
    }

    crate::context_bootstrap::event_target_dispatch::finish_dispatch(scope, event);
    let default_prevented =
        crate::context_bootstrap::event_bool_attribute(scope, event, "defaultPrevented");
    SimpleEventDispatchResult {
        uncanceled: !default_prevented,
        dispatched,
    }
}

pub(crate) struct SimpleEventListenerInvocationResult {
    pub(crate) dispatched: bool,
    pub(crate) did_throw: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn invoke_simple_event_target_listeners<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    slot_name: &str,
    event_type: &str,
    event: v8::Local<'s, v8::Object>,
    capture_phase: bool,
    handler_arguments: &[v8::Local<'s, v8::Value>],
    handler_type: EventHandlerType,
    mut callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
) -> SimpleEventListenerInvocationResult {
    let mut dispatched = false;
    let mut did_throw = false;
    let listeners = simple_object_event_listeners_snapshot(scope, target, slot_name, event_type);
    for listener in listeners
        .iter()
        .filter(|listener| listener.capture == capture_phase)
    {
        let Some(listener) = listener.prepare_for_invocation(scope, target, slot_name, event_type)
        else {
            continue;
        };
        let callback_target = crate::context_bootstrap::shared_event_targets::target_in_realm(
            scope,
            target,
            listener.relevant_context(),
        );
        let Some(callback_event) =
            crate::context_bootstrap::navigation_event_worlds::event_in_realm(
                scope,
                target,
                event,
                listener.relevant_context(),
            )
        else {
            continue;
        };
        let ordinary_arguments = [callback_event.into()];
        let handler_arguments =
            if handler_arguments.len() == 1 && handler_arguments[0].strict_equals(event.into()) {
                ordinary_arguments.as_slice()
            } else {
                handler_arguments
            };
        set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, listener.passive);
        let outcome = invoke_simple_event_listener_collecting_errors(
            scope,
            event_type,
            &format!("simple event target {event_type} listener"),
            &listener,
            callback_target.into(),
            if listener.handler_slot.is_some() {
                handler_arguments
            } else {
                &ordinary_arguments
            },
            callback_event,
            callback_errors.as_deref_mut(),
        );
        dispatched |= outcome.invoked;
        did_throw |= outcome.did_throw;
        if listener.handler_slot.is_some()
            && let Some(returned) = outcome.value
        {
            apply_event_handler_return_value(
                scope,
                event,
                v8::Local::new(scope, &returned),
                handler_type,
            );
        }
        set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);
        if event_stop_immediate_propagation(scope, event) {
            break;
        }
    }
    SimpleEventListenerInvocationResult {
        dispatched,
        did_throw,
    }
}

pub(crate) fn invoke_simple_event_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    callback_name: &str,
    listener: &SimpleObjectEventListenerSnapshot<'s>,
    callback_this: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
    current_event: v8::Local<'s, v8::Object>,
) -> Option<v8::Global<v8::Value>> {
    invoke_simple_event_listener_collecting_errors(
        scope,
        event_type,
        callback_name,
        listener,
        callback_this,
        arguments,
        current_event,
        None,
    )
    .value
}

struct SimpleEventCallbackResult {
    invoked: bool,
    did_throw: bool,
    value: Option<v8::Global<v8::Value>>,
}

#[allow(clippy::too_many_arguments)]
fn invoke_simple_event_listener_collecting_errors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    callback_name: &str,
    listener: &SimpleObjectEventListenerSnapshot<'s>,
    callback_this: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
    current_event: v8::Local<'s, v8::Object>,
    callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
) -> SimpleEventCallbackResult {
    let Some(invocation) = listener.invocation(callback_this, arguments, Some(current_event))
    else {
        return SimpleEventCallbackResult {
            invoked: false,
            did_throw: false,
            value: None,
        };
    };
    // Keep the callback's captured Window identity for every event. A popup
    // aliases its opener's V8 realm, so resolving that realm during dispatch
    // would instead adopt the event target's active popup scope. The captured
    // identity also preserves retirement checks after its realm leaves the
    // live registry.
    invoke_simple_event_callback_with_invocation(
        scope,
        event_type,
        callback_name,
        callback_this,
        listener.relevant_context(),
        listener.relevant_identity(),
        invocation,
        callback_errors,
    )
}

#[allow(clippy::too_many_arguments)]
fn invoke_simple_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    callback_name: &str,
    callback: v8::Local<'s, v8::Object>,
    relevant_context: v8::Local<'s, v8::Context>,
    incumbent_context: v8::Local<'s, v8::Context>,
    is_callable: bool,
    callback_this: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
    current_event: v8::Local<'s, v8::Object>,
    callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
) -> SimpleEventCallbackResult {
    let invocation = CallbackInvocation::new(
        callback,
        callback_this,
        relevant_context,
        incumbent_context,
        is_callable,
        "handleEvent",
        arguments,
        Some(current_event),
    );
    invoke_simple_event_callback_with_invocation(
        scope,
        event_type,
        callback_name,
        callback_this,
        relevant_context,
        None,
        invocation,
        callback_errors,
    )
}

fn simple_event_target_interface_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Value>,
) -> String {
    let constructor_name = v8::Local::<v8::Object>::try_from(target)
        .map(|target| target.get_constructor_name().to_rust_string_lossy(scope))
        .unwrap_or_default();
    if constructor_name == "EventTarget" {
        // Blink constructs the abstract EventTarget interface as an
        // EventTargetImpl and exposes that implementation name to DOMDebugger.
        "EventTargetImpl".to_owned()
    } else {
        constructor_name
    }
}

#[allow(clippy::too_many_arguments)]
fn invoke_simple_event_callback_with_invocation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    callback_name: &str,
    callback_target: v8::Local<'s, v8::Value>,
    relevant_context: v8::Local<'s, v8::Context>,
    captured_relevant_identity: Option<crate::native_bridge::WindowExecutionContextIdentity>,
    mut invocation: CallbackInvocation<'s, '_>,
    callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
) -> SimpleEventCallbackResult {
    let host_ptr = context_host_ptr_from_global_bridge(scope);
    let relevant_identity = captured_relevant_identity.or_else(|| {
        host_ptr.and_then(|host_ptr| {
            unsafe { &*host_ptr }
                .window_execution_context_identity_for_v8_context(scope, relevant_context)
        })
    });
    if let Some(host_ptr) = host_ptr {
        invocation = invocation.with_execution_context_currentness(host_ptr, relevant_identity);
    }
    let _dom_debugger_pause = host_ptr.and_then(|host_ptr| {
        let host = unsafe { &*host_ptr };
        if !host.has_dom_debugger_event_listener_breakpoints() {
            return None;
        }
        let target_name = simple_event_target_interface_name(scope, callback_target);
        host.schedule_dom_debugger_event_listener_pause_for_interface(event_type, &target_name)
    });
    CallbackInvoker::invoke_event_and_then(
        scope,
        "event listener",
        "simple event listener threw",
        CallbackExceptionLogLevel::Debug,
        callback_name,
        invocation,
        |scope, outcome| match outcome {
            CallbackInvocationOutcome::Returned(value) => SimpleEventCallbackResult {
                invoked: true,
                did_throw: false,
                value: Some(value),
            },
            CallbackInvocationOutcome::Threw(report) => {
                if let Some(errors) = callback_errors {
                    errors.push(*report);
                } else if let Some(host_ptr) = host_ptr {
                    report_event_callback_exception(
                        scope,
                        host_ptr,
                        event_type,
                        relevant_identity,
                        None,
                        &report,
                    );
                } else {
                    let _ =
                        crate::worker::dispatch_current_worker_callback_exception(scope, *report);
                }
                SimpleEventCallbackResult {
                    invoked: true,
                    did_throw: true,
                    value: None,
                }
            }
            CallbackInvocationOutcome::Retired => SimpleEventCallbackResult {
                invoked: false,
                did_throw: false,
                value: None,
            },
        },
    )
}
