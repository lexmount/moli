use crate::{document_runtime::EventTargetHandle, dom::native::Node, native_bridge::JsContextHost};

use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::element_attribute;
use super::shared::{EventAttributeHandlerScope, compile_event_attribute_handler_for_owner};

const MESSAGEERROR_EVENT_TYPE: &str = "messageerror";
const ONMESSAGEERROR_ATTRIBUTE: &str = "onmessageerror";

pub(in crate::native_bridge) fn body_onmessageerror_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
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

    if let Some(value) = runtime.registered_event_handler_property_value(
        scope,
        EventTargetHandle::Window,
        MESSAGEERROR_EVENT_TYPE,
    ) {
        rv.set(value);
        return;
    }

    let Some(handler) = compile_body_onmessageerror_attribute(scope, runtime, handle) else {
        rv.set_null();
        return;
    };
    let target_context = scope.get_current_context();
    runtime.set_registered_content_attribute_event_handler_property(
        scope,
        EventTargetHandle::Window,
        MESSAGEERROR_EVENT_TYPE,
        Some(handler),
        target_context,
    );
    rv.set(handler.into());
}

pub(in crate::native_bridge) fn body_onmessageerror_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
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
    let handler = v8::Local::<v8::Function>::try_from(args.get(0)).ok();
    runtime.set_registered_event_handler_property(
        scope,
        EventTargetHandle::Window,
        MESSAGEERROR_EVENT_TYPE,
        handler,
    );
    rv.set_undefined();
}

pub(crate) fn compile_window_body_onmessageerror_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
) -> Option<v8::Local<'s, v8::Function>> {
    let runtime = unsafe { &*host_ptr };
    let document_handle = runtime.document_handle();
    let dom = runtime.dom_host().dom();
    let body_handle = dom
        .node(document_handle)
        .and_then(Node::as_document)
        .and_then(|document| document.body_or_frameset_handle(dom, document_handle))?;
    compile_body_onmessageerror_attribute(scope, host_ptr, body_handle)
}
fn compile_body_onmessageerror_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    body_handle: crate::document_runtime::DomHandle,
) -> Option<v8::Local<'s, v8::Function>> {
    let runtime = unsafe { &*host_ptr };
    let source = element_attribute(runtime, body_handle, ONMESSAGEERROR_ATTRIBUTE)?;
    if source.is_empty() {
        return None;
    }
    let owner = runtime.owner_dispatch_scope_for_node(body_handle)?;
    compile_event_attribute_handler_for_owner(
        scope,
        host_ptr,
        owner,
        &source,
        EventAttributeHandlerScope::Element,
    )
}
