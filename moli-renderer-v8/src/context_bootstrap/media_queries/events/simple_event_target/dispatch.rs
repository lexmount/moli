mod propagation;
pub(crate) use propagation::dispatch_simple_event_target_event_with_ancestors;

use super::*;
use crate::context_bootstrap::shared_event_targets::{ResolvedEventTarget, resolve_event_target};
use crate::event_type::EventTypeKey;
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
    target: ResolvedSimpleEventTarget<'s>,
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

    let dispatch_result = dispatch_resolved_event(
        scope,
        target.identity,
        target.registry,
        &event_type,
        event,
        None,
        None,
        None,
    );
    rv.set_bool(dispatch_result.uncanceled);
}

pub(crate) fn dispatch_simple_event_target_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &(impl EventTypeKey + ?Sized),
    event: v8::Local<'s, v8::Object>,
) -> bool {
    dispatch_simple_event_target_event_collecting_errors(scope, target, event_type, event, None)
        .uncanceled
}

pub(crate) fn dispatch_simple_event_target_event_with_original_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    original_target: v8::Local<'s, v8::Object>,
    event_type: &(impl EventTypeKey + ?Sized),
    event: v8::Local<'s, v8::Object>,
) -> bool {
    dispatch_object_event(
        scope,
        target,
        event_type,
        event,
        Some(original_target),
        None,
        None,
    )
    .uncanceled
}

#[derive(Default)]
pub(crate) struct SimpleEventDispatchResult {
    pub(crate) uncanceled: bool,
    pub(crate) dispatched: bool,
    pub(crate) did_listeners_throw: bool,
}

pub(crate) fn dispatch_simple_event_target_event_collecting_errors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &(impl EventTypeKey + ?Sized),
    event: v8::Local<'s, v8::Object>,
    callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
) -> SimpleEventDispatchResult {
    dispatch_object_event(
        scope,
        target,
        event_type,
        event,
        None,
        callback_errors,
        None,
    )
}

/// Run the caller's microtask checkpoint while the native event is still
/// dispatching. ExtendableEvent.waitUntil and Worker error cancellation depend
/// on this boundary; the shared dispatcher always finishes the event afterward.
pub(crate) fn dispatch_simple_event_target_event_with_checkpoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &(impl EventTypeKey + ?Sized),
    event: v8::Local<'s, v8::Object>,
    checkpoint: fn(&mut v8::PinScope<'s, '_>),
) -> SimpleEventDispatchResult {
    dispatch_object_event(
        scope,
        target,
        event_type,
        event,
        None,
        None,
        Some(checkpoint),
    )
}

fn dispatch_object_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &(impl EventTypeKey + ?Sized),
    event: v8::Local<'s, v8::Object>,
    original_target: Option<v8::Local<'s, v8::Object>>,
    callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
    checkpoint: Option<fn(&mut v8::PinScope<'s, '_>)>,
) -> SimpleEventDispatchResult {
    let Some(target) = resolve_event_target(scope, target) else {
        return SimpleEventDispatchResult {
            uncanceled: true,
            dispatched: false,
            did_listeners_throw: false,
        };
    };
    let Some(registry) = SimpleEventListenerRegistry::from_owner(scope, target.listener_owner)
    else {
        // A native producer may dispatch before listener state is initialized.
        // It still owes the event its target, checkpoint and dispatch cleanup.
        crate::context_bootstrap::event_target_dispatch::begin_dispatch_with_original_target(
            scope,
            target.dispatch_target,
            original_target.unwrap_or(target.dispatch_target),
            event,
        );
        return SimpleEventDispatchResult {
            uncanceled: finish_event_dispatch(scope, event, checkpoint),
            dispatched: false,
            did_listeners_throw: false,
        };
    };
    dispatch_resolved_event(
        scope,
        target,
        registry,
        event_type,
        event,
        original_target,
        callback_errors,
        checkpoint,
    )
}

#[allow(clippy::too_many_arguments)]
fn dispatch_resolved_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: ResolvedEventTarget<'s>,
    registry: SimpleEventListenerRegistry<'s>,
    event_type: &(impl EventTypeKey + ?Sized),
    event: v8::Local<'s, v8::Object>,
    original_target: Option<v8::Local<'s, v8::Object>>,
    callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
    checkpoint: Option<fn(&mut v8::PinScope<'s, '_>)>,
) -> SimpleEventDispatchResult {
    let target = target.dispatch_target;
    let can_invoke =
        crate::context_bootstrap::event_target_dispatch::begin_dispatch_with_original_target(scope, target, original_target.unwrap_or(target), event);
    let mut result = if can_invoke {
        invoke_simple_event_listeners(
            scope,
            target,
            registry,
            event_type,
            event,
            None,
            callback_errors,
        )
    } else {
        SimpleEventDispatchResult::default()
    };
    result.uncanceled = finish_event_dispatch(scope, event, checkpoint);
    result
}

fn invoke_simple_event_listeners<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    registry: SimpleEventListenerRegistry<'s>,
    event_type: &(impl EventTypeKey + ?Sized),
    event: v8::Local<'s, v8::Object>,
    capture: Option<bool>,
    mut callback_errors: Option<&mut Vec<crate::exception_reporting::V8ExceptionReport>>,
) -> SimpleEventDispatchResult {
    let mut dispatched = false;
    let mut did_listeners_throw = false;
    let error_arguments = if event_type.is_type("error")
        && (crate::web_api_interfaces::WorkerGlobalScope::is_instance(scope, target)
            || crate::web_api_interfaces::Window::is_instance(scope, target))
    {
        error_event_handler_arguments(scope, event)
    } else {
        None
    };
    // Snapshot each invocation. Filter capture before preparing a once
    // listener, so another phase cannot consume it prematurely.
    let listeners = simple_object_event_listeners_snapshot_in_registry(scope, registry, event_type);
    for listener in &listeners {
        if capture.is_some_and(|capture| listener.capture != capture) {
            continue;
        }
        let Some(listener) = listener.prepare_in_registry(scope, registry, event_type) else {
            continue;
        };
        set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, listener.passive);
        let callback_target = crate::context_bootstrap::shared_event_targets::target_in_realm(
            scope,
            target,
            listener.relevant_context(),
        );
        let Some(callback_event) = crate::context_bootstrap::event_worlds::event_in_realm(
            scope,
            target,
            event,
            listener.relevant_context(),
        ) else {
            continue;
        };
        let callback_arguments = [callback_event.into()];
        let outcome = invoke_simple_event_listener_collecting_errors(
            scope,
            event_type,
            &format!("simple event target {event_type} listener"),
            &listener,
            callback_target.into(),
            if listener.handler_slot.is_some() {
                error_arguments
                    .as_ref()
                    .map_or(callback_arguments.as_slice(), |values| values.as_slice())
            } else {
                &callback_arguments
            },
            callback_event,
            callback_errors.as_deref_mut(),
        );
        dispatched |= outcome.invoked;
        did_listeners_throw |= outcome.threw;
        if listener.handler_slot.is_some()
            && let Some(returned) = outcome.value
        {
            apply_handler_return_value(
                scope,
                event,
                v8::Local::new(scope, &returned),
                error_arguments.is_some(),
            );
        }
        set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);
        if event_stop_immediate_propagation(scope, event) {
            break;
        }
    }

    SimpleEventDispatchResult {
        uncanceled: true,
        dispatched,
        did_listeners_throw,
    }
}

fn finish_event_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    checkpoint: Option<fn(&mut v8::PinScope<'s, '_>)>,
) -> bool {
    if let Some(checkpoint) = checkpoint {
        checkpoint(scope);
    }
    crate::context_bootstrap::event_target_dispatch::finish_dispatch(scope, event);
    !crate::context_bootstrap::event_bool_attribute(scope, event, "defaultPrevented")
}

struct SimpleEventCallbackResult {
    invoked: bool,
    threw: bool,
    value: Option<v8::Global<v8::Value>>,
}

#[allow(clippy::too_many_arguments)]
fn invoke_simple_event_listener_collecting_errors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &(impl EventTypeKey + ?Sized),
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
    // Keep the callback's captured Window identity for every event, including
    // retirement checks after its realm leaves the live registry.
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
    event_type: &(impl EventTypeKey + ?Sized),
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
                threw: false,
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
                    let _ = crate::worker::dispatch_current_worker_callback_exception(scope, *report);
                }
                SimpleEventCallbackResult {
                    invoked: true,
                    threw: true,
                    value: None,
                }
            }
            CallbackInvocationOutcome::Retired => SimpleEventCallbackResult {
                invoked: false,
                threw: false,
                value: None,
            },
        },
    )
}

fn apply_handler_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    returned: v8::Local<'s, v8::Value>,
    error_handler: bool,
) {
    let handler_type = if error_handler {
        EventHandlerType::OnErrorEventHandler
    } else {
        EventHandlerType::EventHandler
    };
    apply_event_handler_return_value(scope, event, returned, handler_type);
}
