use crate::{
    context_bootstrap::{
        set_window_body_onerror_handler_compiled, set_window_onerror_handler_value,
        window_body_onerror_handler_is_compiled,
    },
    util::{get_private_value, set_private_value, v8_string, v8str},
};

use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::element_attribute;
use super::shared::compile_event_attribute_handler;

const BODY_ONLOAD_COMPILED_SLOT: &str = "__moliBodyOnloadCompiled";

/// Resolve native inline Window source at dispatch, without observing the
/// page's document/body/onload JavaScript getters.
pub(crate) fn compile_window_body_event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    let host = unsafe { &*host_ptr };
    let handle = host.event_handler_content_attribute_node(
        crate::document_runtime::EventTargetHandle::Window,
        event_type,
    )?;
    let name = super::generic::event_handler_content_attribute_name(event_type)?;
    if event_type == "load" && body_window_event_handler_is_compiled(scope, "onload") == Some(true)
    {
        return None;
    }
    let source = element_attribute(host, handle, &name)?;
    if event_type == "load" {
        set_body_window_event_handler_compiled(scope, "onload", true);
    }
    // A parse error can report another event synchronously. Do not recompile
    // the same handler while its original compilation is in progress.
    unsafe { &mut *host_ptr }.set_registered_event_handler_property(
        scope,
        crate::document_runtime::EventTargetHandle::Window,
        event_type,
        None,
    );
    let event = v8_string(scope, "event")?;
    let handler = compile_event_attribute_handler(scope, host_ptr, handle, &source, &[event], &[]);
    if let Some(handler) = handler
        && let Some(name) = v8_string(scope, &name)
    {
        handler.set_name(name);
    }
    let context = scope.get_current_context();
    unsafe { &mut *host_ptr }.set_registered_content_attribute_event_handler_property(
        scope,
        crate::document_runtime::EventTargetHandle::Window,
        event_type,
        handler,
        context,
    );
    handler
}

fn body_window_event_handler_is_compiled(
    scope: &mut v8::PinScope<'_, '_>,
    handler_name: &str,
) -> Option<bool> {
    match handler_name {
        "onload" => {
            let global = scope.get_current_context().global(scope);
            Some(
                get_private_value(scope, global, BODY_ONLOAD_COMPILED_SLOT)
                    .is_some_and(|value| value.boolean_value(scope)),
            )
        }
        "onerror" => Some(window_body_onerror_handler_is_compiled(scope)),
        _ => None,
    }
}

fn set_body_window_event_handler_compiled(
    scope: &mut v8::PinScope<'_, '_>,
    handler_name: &str,
    compiled: bool,
) -> bool {
    match handler_name {
        "onload" => {
            let global = scope.get_current_context().global(scope);
            let compiled = v8::Boolean::new(scope, compiled);
            set_private_value(scope, global, BODY_ONLOAD_COMPILED_SLOT, compiled.into());
            true
        }
        "onerror" => {
            set_window_body_onerror_handler_compiled(scope, compiled);
            true
        }
        _ => false,
    }
}

pub(in crate::native_bridge) fn body_onload_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    body_window_event_handler_getter(scope, args, rv, "onload", &["event"]);
}

pub(in crate::native_bridge) fn body_onerror_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    body_window_event_handler_getter(
        scope,
        args,
        rv,
        "onerror",
        &["event", "source", "lineno", "colno", "error"],
    );
}

fn body_window_event_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    handler_name: &'static str,
    argument_names: &[&str],
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if !super::is_body_or_frameset_element(runtime, handle) {
        rv.set_null();
        return;
    }

    if let Some(value) = body_window_event_handler(scope, handler_name)
        && value.is_function()
    {
        rv.set(value);
        return;
    }

    let Some(compiled) = body_window_event_handler_is_compiled(scope, handler_name) else {
        rv.set(v8::null(scope).into());
        return;
    };
    if compiled {
        rv.set(v8::null(scope).into());
        return;
    }

    let Some(source) = element_attribute(runtime, handle, handler_name) else {
        rv.set(v8::null(scope).into());
        return;
    };
    if source.is_empty() {
        rv.set(v8::null(scope).into());
        return;
    }
    let _ = set_body_window_event_handler_compiled(scope, handler_name, true);
    body_window_event_handler_set(scope, handler_name, v8::null(scope).into());
    let arguments = argument_names
        .iter()
        .filter_map(|name| v8_string(scope, name))
        .collect::<Vec<_>>();
    if arguments.len() != argument_names.len() {
        rv.set(v8::null(scope).into());
        return;
    }
    if let Some(handler) =
        compile_event_attribute_handler(scope, runtime_ptr, handle, &source, &arguments, &[])
    {
        handler.set_name(v8str(scope, handler_name));
        body_window_event_handler_set(scope, handler_name, handler.into());
        rv.set(handler.into());
        return;
    }
    body_window_event_handler_set(scope, handler_name, v8::null(scope).into());
    rv.set(v8::null(scope).into());
}

pub(in crate::native_bridge) fn body_onload_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    body_window_event_handler_setter(scope, args, rv, "onload");
}

pub(in crate::native_bridge) fn body_onerror_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    body_window_event_handler_setter(scope, args, rv, "onerror");
}

fn body_window_event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    handler_name: &'static str,
) {
    let _ = set_body_window_event_handler_compiled(scope, handler_name, true);
    let value = args.get(0);
    if value.is_function() {
        body_window_event_handler_set(scope, handler_name, value);
    } else {
        body_window_event_handler_set(scope, handler_name, v8::null(scope).into());
    }
    rv.set_undefined();
}

fn body_window_event_handler<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handler_name: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    global.get(scope, v8str(scope, handler_name).into())
}

fn body_window_event_handler_set(
    scope: &mut v8::PinScope<'_, '_>,
    handler_name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    if handler_name == "onerror" {
        set_window_onerror_handler_value(scope, value);
        return;
    }
    let global = scope.get_current_context().global(scope);
    let _ = global.set(scope, v8str(scope, handler_name).into(), value);
}

pub(crate) fn initialize_parser_inserted_body_window_event_handlers(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    if !super::is_body_or_frameset_element(runtime, handle) {
        return;
    }
    for name in ["onload", "onerror"] {
        if runtime.dom_host().get_attribute(handle, name).is_some() {
            invalidate_body_window_event_attribute_handler(scope, runtime, handle, name);
        }
    }
}

pub(super) fn invalidate_body_window_event_attribute_handler(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    name: &str,
) {
    if !super::is_body_or_frameset_element(runtime, handle) {
        return;
    }
    let normalized_name = name.to_ascii_lowercase();
    let handler_name = match normalized_name.as_str() {
        "onload" => "onload",
        "onerror" => "onerror",
        _ => return,
    };
    let _ = set_body_window_event_handler_compiled(scope, handler_name, false);
    if handler_name == "onload"
        && runtime.owner_dispatch_scope_for_node(handle)
            == Some(crate::native_bridge::OwnerDispatchScope::Top)
    {
        // Absence means uncompiled content; explicit null is a disabled handler.
        runtime.clear_event_handler_property(
            crate::document_runtime::EventTargetHandle::Window,
            "load",
        );
        return;
    }
    body_window_event_handler_set(scope, handler_name, v8::null(scope).into());
}
