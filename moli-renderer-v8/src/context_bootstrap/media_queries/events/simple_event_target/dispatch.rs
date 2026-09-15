use super::*;
use crate::{
    callback_invocation::{CallbackInvocation, CallbackInvocationOutcome, CallbackInvoker},
    context_bootstrap::events::{
        EVENT_PASSIVE_SLOT, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, clear_event_composed_path,
        clear_event_dispatch_fields, event_internal_bool_flag, set_event_composed_path,
        set_event_dispatch_fields, set_event_internal_flag,
    },
    context_bootstrap::{EventHandlerType, apply_event_handler_return_value},
    exception_reporting::CallbackExceptionLogLevel,
    host::report_event_callback_exception,
    native_bridge::lightweight_popup_id_from_window,
    util::{context_host_ptr_from_global_bridge, serialize_v8_array},
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
    let event_value = args.get(0);
    if !event_value.is_object() || event_value.is_function() {
        throw_type_error(
            scope,
            "Failed to execute 'dispatchEvent': parameter 1 is not an object.",
        );
        return;
    }
    let Ok(event) = v8::Local::<v8::Object>::try_from(event_value) else {
        throw_type_error(
            scope,
            "Failed to execute 'dispatchEvent': parameter 1 is not an object.",
        );
        return;
    };
    let Some(event_type) = object_string_property_defined(scope, event, "type") else {
        throw_type_error(
            scope,
            "Failed to execute 'dispatchEvent': event type is required.",
        );
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
    set_event_dispatch_fields(scope, target, event);
    let path = serialize_v8_array(scope, [target]).unwrap_or_else(|| v8::Array::new(scope, 1));
    set_event_composed_path(scope, event, path);

    if !simple_event_target_uses_ordered_handlers(scope, target) {
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
            let returned = invoke_simple_event_callback(
                scope,
                event_type,
                &format!("simple event target {handler_name}"),
                handler,
                relevant_context,
                incumbent_context,
                true,
                target.into(),
                &[event.into()],
                event,
            );
            if let Some(returned) = returned {
                apply_event_handler_return_value(
                    scope,
                    event,
                    v8::Local::new(scope, &returned),
                    EventHandlerType::EventHandler,
                );
            }
        }
    }

    if !event_stop_immediate_propagation(scope, event) {
        let listeners =
            simple_object_event_listeners_snapshot(scope, target, slot_name, event_type);
        'phases: for capture_phase in [true, false] {
            for listener in listeners
                .iter()
                .filter(|listener| listener.capture == capture_phase)
            {
                if !simple_object_event_listener_is_registered(
                    scope,
                    target,
                    slot_name,
                    event_type,
                    listener.original,
                    listener.capture,
                ) {
                    continue;
                }
                if listener.once {
                    simple_object_event_remove_listener_value_for_type(
                        scope,
                        target,
                        slot_name,
                        event_type,
                        listener.original,
                        listener.capture,
                    );
                }
                set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, listener.passive);
                let returned = invoke_simple_event_listener(
                    scope,
                    event_type,
                    &format!("simple event target {event_type} listener"),
                    listener,
                    target.into(),
                    &[event.into()],
                    event,
                );
                if listener.handler_slot.is_some()
                    && let Some(returned) = returned
                {
                    apply_event_handler_return_value(
                        scope,
                        event,
                        v8::Local::new(scope, &returned),
                        EventHandlerType::EventHandler,
                    );
                }
                set_event_internal_flag(scope, event, EVENT_PASSIVE_SLOT, false);
                if event_stop_immediate_propagation(scope, event) {
                    break 'phases;
                }
            }
        }
    }

    clear_event_dispatch_fields(scope, event);
    clear_event_composed_path(scope, event);
    let default_prevented = object_bool_property(scope, event, "defaultPrevented").unwrap_or(false);
    !default_prevented
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
    let invocation = listener.invocation(callback_this, arguments, Some(current_event));
    // Lightweight popup Window shells alias the opener's concrete V8 realm.
    // Retain the callback's exact registration-time Window only for popup
    // `load`: this is where a top-realm WPT callback must keep scheduling work
    // on the opener after it calls `popup.close()`. Other synthetic popup
    // events deliberately execute in their target owner scope until those
    // Window shells gain distinct V8 realms.
    let captured_relevant_identity = if event_type == "load"
        && v8::Local::<v8::Object>::try_from(callback_this)
            .ok()
            .and_then(|target| lightweight_popup_id_from_window(scope, target))
            .is_some()
    {
        listener.relevant_identity().filter(|identity| {
            !matches!(
                identity.dispatch_scope(),
                crate::native_bridge::OwnerDispatchScope::LightweightPopup(_)
            )
        })
    } else {
        None
    };
    invoke_simple_event_callback_with_invocation(
        scope,
        event_type,
        callback_name,
        callback_this,
        listener.relevant_context(),
        captured_relevant_identity,
        invocation,
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
) -> Option<v8::Global<v8::Value>> {
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

fn invoke_simple_event_callback_with_invocation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    callback_name: &str,
    callback_target: v8::Local<'s, v8::Value>,
    relevant_context: v8::Local<'s, v8::Context>,
    captured_relevant_identity: Option<crate::native_bridge::WindowExecutionContextIdentity>,
    mut invocation: CallbackInvocation<'s, '_>,
) -> Option<v8::Global<v8::Value>> {
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
            CallbackInvocationOutcome::Returned(value) => Some(value),
            CallbackInvocationOutcome::Threw(report) => {
                if let Some(host_ptr) = host_ptr {
                    report_event_callback_exception(
                        scope,
                        host_ptr,
                        event_type,
                        relevant_identity,
                        None,
                        &report,
                    );
                }
                None
            }
            CallbackInvocationOutcome::Retired => None,
        },
    )
}
