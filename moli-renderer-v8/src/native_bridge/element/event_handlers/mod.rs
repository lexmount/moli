mod body_onload;
mod body_window;
mod generic;
mod shared;

pub(crate) use body_onload::compile_window_body_event_attribute;
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
    GlobalEventHandlerOwner, compile_node_content_attribute_for_dispatch,
    event_handler_content_attribute_name, install_global_event_handler_template_bindings,
    install_touch_event_handler_realm_bindings, is_global_event_handler_attribute,
    node_event_handler_getter_function, node_event_handler_setter_function,
    node_event_handler_value_from_wrapper,
};
pub(crate) use shared::{EventAttributeHandlerScope, compile_event_attribute_handler_for_owner};

pub(crate) fn is_body_window_event_handler_attribute(name: &str) -> bool {
    matches!(
        name,
        "onafterprint"
            | "onbeforeprint"
            | "onbeforeunload"
            | "onblur"
            | "onerror"
            | "onfocus"
            | "onhashchange"
            | "onlanguagechange"
            | "onload"
            | "onmessage"
            | "onmessageerror"
            | "onoffline"
            | "ononline"
            | "onpagehide"
            | "onpageshow"
            | "onpopstate"
            | "onrejectionhandled"
            | "onresize"
            | "onscroll"
            | "onstorage"
            | "onunhandledrejection"
            | "onunload"
    )
}

fn is_body_or_frameset_element(
    runtime: &super::super::PageScriptHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    runtime.dom_host().node(handle).is_some_and(|node| {
        node.is_html_element_named("body") || node.is_html_element_named("frameset")
    })
}

pub(crate) fn invalidate_event_handler_content_attribute(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut super::super::PageScriptHost,
    handle: crate::document_runtime::DomHandle,
    name: &str,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    generic::invalidate_event_attribute_handler(scope, runtime, handle, name);
    body_onload::invalidate_body_window_event_attribute_handler(scope, runtime, handle, name);
}
