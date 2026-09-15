use crate::{
    context_bootstrap::BODY_OR_FRAMESET_WINDOW_EVENT_HANDLER_PROPERTIES,
    document_runtime::{DomHandle, EventTargetHandle},
    native_bridge::{JsContextHost, OwnerDispatchScope},
    util::{v8_string, v8str},
};

use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::element_attribute;
use super::shared::compile_event_attribute_handler;

fn body_or_frameset_window_event_handler_properties() -> impl Iterator<Item = &'static str> {
    BODY_OR_FRAMESET_WINDOW_EVENT_HANDLER_PROPERTIES
        .iter()
        .copied()
}

pub(crate) fn body_or_frameset_reflects_window_event_type(event_type: &str) -> bool {
    body_or_frameset_window_event_handler_properties()
        .any(|name| name.strip_prefix("on") == Some(event_type))
}

pub(crate) fn install_body_or_frameset_window_event_handler_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    for name in body_or_frameset_window_event_handler_properties() {
        let data = v8str(scope, name).into();
        let getter = v8::FunctionTemplate::builder(body_window_event_handler_getter_function)
            .data(data)
            .length(0)
            .build(scope);
        let setter = v8::FunctionTemplate::builder(body_window_event_handler_setter_function)
            .data(data)
            .length(1)
            .build(scope);
        if let Some(function_name) = v8_string(scope, &format!("get {name}")) {
            getter.set_class_name(function_name);
        }
        if let Some(function_name) = v8_string(scope, &format!("set {name}")) {
            setter.set_class_name(function_name);
        }
        prototype.set_accessor_property(
            v8str(scope, name).into(),
            Some(getter),
            Some(setter),
            v8::PropertyAttribute::NONE,
        );
    }
}

fn body_window_event_handler_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler_name) = handler_name_from_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let Some(event_type) = handler_name.strip_prefix("on") else {
        rv.set_null();
        return;
    };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = match super::body_or_frameset_window_owner(unsafe { &*runtime_ptr }, handle) {
        Some(OwnerDispatchScope::Top) => {
            resolve_window_event_handler_content_attribute(scope, runtime_ptr, event_type)
        }
        Some(OwnerDispatchScope::Child(child_handle)) => unsafe { &*runtime_ptr }
            .child_window_event_handler_property_value(scope, child_handle, &handler_name),
        Some(OwnerDispatchScope::LightweightPopup(_)) | None => None,
    };
    match value {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

fn body_window_event_handler_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(handler_name) = handler_name_from_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(event_type) = handler_name.strip_prefix("on") else {
        rv.set_undefined();
        return;
    };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let handler = v8::Local::<v8::Function>::try_from(args.get(0)).ok();
    match super::body_or_frameset_window_owner(unsafe { &*runtime_ptr }, handle) {
        Some(OwnerDispatchScope::Top) => unsafe { &mut *runtime_ptr }
            .set_registered_event_handler_property(
                scope,
                EventTargetHandle::Window,
                event_type,
                handler,
            ),
        Some(OwnerDispatchScope::Child(child_handle)) => {
            let relevant_context = handler
                .and_then(|handler| handler.get_creation_context(scope))
                .unwrap_or_else(|| scope.get_current_context());
            unsafe { &mut *runtime_ptr }.set_child_window_event_handler_property(
                scope,
                child_handle,
                &handler_name,
                handler,
                relevant_context,
            );
        }
        Some(OwnerDispatchScope::LightweightPopup(_)) | None => {}
    }
    rv.set_undefined();
}

pub(crate) fn resolve_window_event_handler_content_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let runtime = unsafe { &mut *runtime_ptr };
    if let Some(value) = runtime.registered_event_handler_property_value(
        scope,
        EventTargetHandle::Window,
        event_type,
    ) {
        return Some(value);
    }
    let owner = runtime
        .uncompiled_event_handler_content_attribute_owner(EventTargetHandle::Window, event_type)?;
    if !super::body_or_frameset_uses_runtime_window(runtime, owner) {
        return None;
    }

    // Compilation can report an error, which dispatches a Window `error`
    // event and re-enters the corresponding getter. Replace the uncompiled
    // state before invoking V8 so that re-entry observes null instead of
    // recursively compiling the same content attribute.
    let target_context = scope.get_current_context();
    runtime.set_registered_content_attribute_event_handler_property(
        scope,
        EventTargetHandle::Window,
        event_type,
        None,
        target_context,
    );
    let handler = compile_body_window_event_attribute(scope, runtime_ptr, owner, event_type);
    let target_context = scope.get_current_context();
    unsafe { &mut *runtime_ptr }.set_registered_content_attribute_event_handler_property(
        scope,
        EventTargetHandle::Window,
        event_type,
        handler,
        target_context,
    );
    Some(match handler {
        Some(handler) => handler.into(),
        None => v8::null(scope).into(),
    })
}

fn compile_body_window_event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    owner: DomHandle,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    let handler_name = format!("on{event_type}");
    let source = element_attribute(unsafe { &*runtime_ptr }, owner, &handler_name)?;
    if source.is_empty() {
        return None;
    }
    let argument_names: &[&str] = if event_type == "error" {
        &["event", "source", "lineno", "colno", "error"]
    } else {
        &["event"]
    };
    let arguments = argument_names
        .iter()
        .filter_map(|name| v8_string(scope, name))
        .collect::<Vec<_>>();
    if arguments.len() != argument_names.len() {
        return None;
    }
    let handler =
        compile_event_attribute_handler(scope, runtime_ptr, owner, &source, &arguments, &[])?;
    if let Some(name) = v8_string(scope, &handler_name) {
        handler.set_name(name);
    }
    Some(handler)
}

pub(crate) fn initialize_parser_inserted_body_window_event_handlers(
    _scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    if !super::body_or_frameset_uses_runtime_window(runtime, handle) {
        return;
    }
    for handler_name in body_or_frameset_window_event_handler_properties() {
        let event_type = handler_name
            .strip_prefix("on")
            .expect("body Window event handler name must start with on");
        if runtime
            .dom_host()
            .get_attribute(handle, handler_name)
            .is_some()
            && let Some(previous) = runtime.set_event_handler_content_attribute(
                EventTargetHandle::Window,
                event_type,
                Some(handle),
            )
        {
            runtime.release_event_callback(previous);
        }
    }
}

fn handler_name_from_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<String> {
    v8::Local::<v8::String>::try_from(data)
        .ok()
        .map(|name| name.to_rust_string_lossy(scope))
}
