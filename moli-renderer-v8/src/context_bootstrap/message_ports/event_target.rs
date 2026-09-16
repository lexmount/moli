use super::*;
use crate::event_listener_args::{AddEventListenerArgs, RemoveEventListenerArgs};
use crate::webidl;

pub(in crate::context_bootstrap) fn message_port_add_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AddEventListenerArgs>(scope, &args) else {
        return;
    };
    if !message_port_supports_event_type(&parsed.event_type) {
        rv.set_undefined();
        return;
    }
    let Some(listener) = parsed.listener else {
        rv.set_undefined();
        return;
    };
    let options = parsed.options.options;
    let signal = parsed.options.signal;
    if signal.is_some_and(|signal| signal.is_aborted(scope)) {
        rv.set_undefined();
        return;
    }
    let Some(port_id) = message_port_id_from_object(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    let Some(listener_id) = register_message_port_event_listener(
        scope,
        args.this(),
        parsed.event_type,
        listener,
        options,
    ) else {
        rv.set_undefined();
        return;
    };
    if let Some(signal) = signal
        && !signal.register_message_port_listener(scope, port_id, listener_id)
    {
        remove_message_port_event_listener_by_id(scope, port_id, listener_id);
    }
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn message_port_remove_event_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RemoveEventListenerArgs>(scope, &args) else {
        return;
    };
    if !message_port_supports_event_type(&parsed.event_type) {
        rv.set_undefined();
        return;
    }
    let Some(listener) = parsed.listener else {
        rv.set_undefined();
        return;
    };
    let capture = parsed.options.capture;
    remove_message_port_event_listener(scope, args.this(), &parsed.event_type, &listener, capture);
    rv.set_undefined();
}

fn message_port_supports_event_type(event_type: &str) -> bool {
    matches!(event_type, "message" | "messageerror" | "close")
}
