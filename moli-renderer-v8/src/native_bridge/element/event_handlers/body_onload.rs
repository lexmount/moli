use crate::{
    document_runtime::EventTargetHandle,
    dom::native::Node,
    util::{v8_string, v8str},
};

use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::element_attribute;
use super::shared::compile_event_attribute_handler;

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

    let event_type = handler_name
        .strip_prefix("on")
        .expect("body window event handler name must start with on");
    if let Some(value) = runtime.registered_event_handler_property_value(
        scope,
        EventTargetHandle::Window,
        event_type,
    ) {
        rv.set(value);
        return;
    }

    if element_attribute(runtime, handle, handler_name).is_none() {
        rv.set(v8::null(scope).into());
        return;
    }
    let handler = compile_body_window_event_attribute(
        scope,
        runtime_ptr,
        handle,
        handler_name,
        argument_names,
    );
    let target_context = scope.get_current_context();
    runtime.set_registered_content_attribute_event_handler_property(
        scope,
        EventTargetHandle::Window,
        event_type,
        handler,
        target_context,
    );
    match handler {
        Some(handler) => rv.set(handler.into()),
        None => rv.set(v8::null(scope).into()),
    }
}

pub(crate) fn compile_window_body_onload_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
) -> Option<v8::Local<'s, v8::Function>> {
    let runtime = unsafe { &*host_ptr };
    let document_handle = runtime.document_handle();
    let dom = runtime.dom_host().dom();
    let body_handle = dom
        .node(document_handle)
        .and_then(Node::as_document)
        .and_then(|document| document.body_or_frameset_handle(dom, document_handle))?;
    compile_body_window_event_attribute(scope, host_ptr, body_handle, "onload", &["event"])
}

fn compile_body_window_event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    handler_name: &'static str,
    argument_names: &[&str],
) -> Option<v8::Local<'s, v8::Function>> {
    let runtime = unsafe { &*runtime_ptr };
    let source = element_attribute(runtime, handle, handler_name)?;
    if source.is_empty() {
        return None;
    }
    let arguments = argument_names
        .iter()
        .filter_map(|name| v8_string(scope, name))
        .collect::<Vec<_>>();
    if arguments.len() != argument_names.len() {
        return None;
    }
    let handler =
        compile_event_attribute_handler(scope, runtime_ptr, handle, &source, &arguments, &[]);
    if let Some(handler) = handler {
        handler.set_name(v8str(scope, handler_name));
    }
    handler
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
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if !super::is_body_or_frameset_element(runtime, handle) {
        rv.set_undefined();
        return;
    }
    let event_type = handler_name
        .strip_prefix("on")
        .expect("body window event handler name must start with on");
    runtime.set_registered_event_handler_property(
        scope,
        EventTargetHandle::Window,
        event_type,
        v8::Local::<v8::Function>::try_from(args.get(0)).ok(),
    );
    rv.set_undefined();
}

pub(crate) fn initialize_parser_inserted_body_window_event_handlers(
    _scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    if !super::is_body_or_frameset_element(runtime, handle) {
        return;
    }
    for event_type in ["load", "error", "messageerror"] {
        if runtime
            .dom_host()
            .get_attribute(handle, &format!("on{event_type}"))
            .is_some()
        {
            runtime.set_event_handler_content_attribute(
                EventTargetHandle::Window,
                event_type,
                true,
            );
        }
    }
}
