use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use moli_web_mime::{data_url_mime_type, is_image_mime};

pub(in crate::native_bridge) fn image_complete_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_bool(image_complete_value(scope, args.this()));
}

fn image_complete_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return true;
    };
    image_complete_value_for_handle(unsafe { &*runtime_ptr }, handle)
}

fn image_complete_value_for_handle(
    runtime: &crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> bool {
    let Some(element) = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
    else {
        return true;
    };

    let src = element.attribute("src");
    let srcset = element.attribute("srcset");
    match (src, srcset) {
        (None, None) => true,
        (Some(""), None) => true,
        (Some(src), _) if data_url_src_is_image(src) => true,
        _ => runtime
            .pending_image_load_event(handle)
            .filter(|pending| runtime.pending_image_load_event_is_current(handle, pending))
            .map_or_else(
                || element.image_load_dispatched() || runtime.image_resource_is_ready(handle),
                |pending| pending.terminal_source().is_some(),
            ),
    }
}

fn data_url_src_is_image(src: &str) -> bool {
    data_url_mime_type(src.trim_start()).is_some_and(|mime| is_image_mime(&mime))
}
