use super::*;

#[derive(Clone, Copy)]
pub(crate) enum EventHandlerType {
    EventHandler,
    OnErrorEventHandler,
}

pub(super) fn set_event_default_prevented(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) {
    let key = v8str(scope, "defaultPrevented");
    let value = v8::Boolean::new(scope, true).into();
    let _ = event.define_own_property(scope, key.into(), value, Default::default());
}

fn prevent_event_default_if_allowed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    if !object_bool_property(scope, event, "cancelable").unwrap_or(false)
        || event_internal_bool_flag(scope, event, EVENT_PASSIVE_SLOT)
    {
        return;
    }
    set_event_default_prevented(scope, event);
}

pub(crate) fn apply_event_handler_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    returned: v8::Local<'s, v8::Value>,
    handler_type: EventHandlerType,
) {
    let cancels = match handler_type {
        EventHandlerType::EventHandler => returned.is_boolean() && !returned.boolean_value(scope),
        EventHandlerType::OnErrorEventHandler => returned.is_true(),
    };
    if cancels {
        prevent_event_default_if_allowed(scope, event);
    }
}

pub(crate) fn apply_before_unload_event_handler_return_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    returned: v8::Local<'s, v8::String>,
) {
    if !event_is_before_unload_event(scope, event) {
        return;
    }
    set_event_default_prevented(scope, event);
    if before_unload_event_return_value_is_empty(scope, event) {
        set_before_unload_event_return_value(scope, event, returned);
    }
}

pub(in crate::context_bootstrap) fn event_prevent_default_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    prevent_event_default_if_allowed(scope, event);
}

pub(in crate::context_bootstrap) fn event_return_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_initialized(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let default_prevented =
        object_bool_property(scope, args.this(), "defaultPrevented").unwrap_or(false);
    rv.set(v8::Boolean::new(scope, !default_prevented).into());
}

pub(in crate::context_bootstrap) fn event_return_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    if event_initialized(scope, event).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if args.get(0).boolean_value(scope)
        || !object_bool_property(scope, event, "cancelable").unwrap_or(false)
        || event_internal_bool_flag(scope, event, EVENT_PASSIVE_SLOT)
    {
        return;
    }
    set_event_default_prevented(scope, event);
}

pub(in crate::context_bootstrap) fn event_cancel_bubble_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_initialized(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let stopped = event_internal_bool_flag(scope, args.this(), EVENT_STOP_PROPAGATION_SLOT);
    rv.set(v8::Boolean::new(scope, stopped).into());
}

pub(in crate::context_bootstrap) fn event_cancel_bubble_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if event_initialized(scope, args.this()).is_none() {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if !args.get(0).boolean_value(scope) {
        return;
    }
    set_event_internal_flag(scope, args.this(), EVENT_STOP_PROPAGATION_SLOT, true);
}

pub(in crate::context_bootstrap) fn event_stop_propagation_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    set_event_internal_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT, true);
}

pub(in crate::context_bootstrap) fn event_stop_immediate_propagation_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = args.this();
    set_event_internal_flag(scope, event, EVENT_STOP_PROPAGATION_SLOT, true);
    set_event_internal_flag(scope, event, EVENT_STOP_IMMEDIATE_PROPAGATION_SLOT, true);
}

pub(in crate::context_bootstrap) fn event_composed_path_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = super::base::event_composed_path_value(scope, args.this());
    rv.set(value);
}

pub(in crate::context_bootstrap) fn event_time_stamp_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let time_stamp = super::base::event_time_stamp(scope, args.this());
    rv.set(v8::Number::new(scope, time_stamp).into());
}
