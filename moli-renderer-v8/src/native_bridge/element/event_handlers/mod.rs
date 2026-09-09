mod body_onload;
mod body_window;
mod generic;
mod shared;

pub(crate) use body_onload::initialize_parser_inserted_body_window_event_handlers;
pub(super) use body_onload::{
    body_onerror_getter_function, body_onerror_setter_function, body_onload_getter_function,
    body_onload_setter_function,
};
pub(crate) use body_window::compile_window_body_onmessageerror_attribute;
pub(super) use body_window::{
    body_onmessageerror_getter_function, body_onmessageerror_setter_function,
};
pub(crate) use generic::{
    GlobalEventHandlerOwner, install_global_event_handler_template_bindings,
    install_touch_event_handler_realm_bindings,
};
pub(crate) use shared::{EventAttributeHandlerScope, compile_event_attribute_handler_for_owner};

fn is_body_or_frameset_element(
    runtime: &super::super::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    runtime.dom_host().node(handle).is_some_and(|node| {
        node.is_html_element_named("body") || node.is_html_element_named("frameset")
    })
}

pub(crate) fn invalidate_event_handler_content_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut super::super::JsContextHost,
    handle: crate::document_runtime::DomHandle,
    name: &str,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    generic::invalidate_node_event_attribute_handler(runtime, handle, name);
    body_onload::invalidate_body_window_event_attribute_handler(scope, runtime, handle, name);
}
