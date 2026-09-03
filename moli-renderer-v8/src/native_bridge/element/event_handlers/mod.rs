mod body_window;
mod generic;
mod shared;

pub(crate) use body_window::{
    body_or_frameset_reflects_window_event_type,
    initialize_parser_inserted_body_window_event_handlers,
    install_body_or_frameset_window_event_handler_accessors,
    resolve_window_event_handler_content_attribute,
};
pub(super) use generic::{
    ELEMENT_FULLSCREEN_EVENT_HANDLER_PROPERTIES, is_element_event_handler_content_attribute_name,
};
pub(crate) use generic::{
    GlobalEventHandlerOwner, canonical_event_handler_event_type,
    event_handler_content_attribute_name, install_global_event_handler_template_bindings,
    install_node_event_handler_template_bindings, node_event_handler_getter_function,
    node_event_handler_setter_function,
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

pub(super) fn body_or_frameset_window_owner(
    runtime: &super::super::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    if !is_body_or_frameset_element(runtime, handle) {
        return None;
    }
    match runtime.owner_dispatch_scope_for_node(handle)? {
        owner @ (crate::native_bridge::OwnerDispatchScope::Top
        | crate::native_bridge::OwnerDispatchScope::Child(_)) => Some(owner),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(_) => None,
    }
}

pub(crate) fn body_or_frameset_uses_runtime_window(
    runtime: &super::super::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    body_or_frameset_window_owner(runtime, handle)
        == Some(crate::native_bridge::OwnerDispatchScope::Top)
}
