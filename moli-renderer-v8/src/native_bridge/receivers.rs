//! Native DOM brand predicates for declarative bindings. Do not consult public
//! constructors or prototype chains: those are mutable, realm-specific JS state.

use super::node::{node_is_document, node_runtime_and_handle_from_object_or_detached};

pub(crate) fn document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .ok()
        .is_some_and(|(runtime, handle)| node_is_document(unsafe { &*runtime }, handle))
}

pub(crate) fn html_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .ok()
        .is_some_and(|(runtime, handle)| {
            unsafe { &*runtime }
                .dom_host()
                .node(handle)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.namespace() == super::document::XHTML_NS)
        })
}

pub(crate) fn html_canvas_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    html_element_named(scope, receiver, "canvas")
}

pub(crate) fn html_iframe_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    html_element_named(scope, receiver, "iframe")
}

fn html_element_named<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    local_name: &str,
) -> bool {
    node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .ok()
        .is_some_and(|(runtime, handle)| {
            unsafe { &*runtime }
                .dom_host()
                .is_html_element_named(handle, local_name)
        })
}
