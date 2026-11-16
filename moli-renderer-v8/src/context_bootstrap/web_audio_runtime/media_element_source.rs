use super::*;
pub(in crate::context_bootstrap) use moli_webapi::web_audio::media_element_source::{
    constructor, create,
};

pub(super) fn initialize_native_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
    element: v8::Local<'s, v8::Object>,
) -> bool {
    let Ok((host, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, element)
    else {
        throw_type_error(scope, "The media element has no native backing.");
        return false;
    };
    let already_connected = unsafe { &*host }
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(crate::dom::native::Element::media_audio_source_created);
    if already_connected {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The media element is already connected to an audio source node.",
        );
        return false;
    }
    if !moli_webapi::web_audio::media_element_source::bind_native_source(
        scope, node, context, element,
    ) {
        return false;
    }
    if let Some(element) = unsafe { &mut *host }
        .dom_host_mut()
        .node_mut(handle)
        .and_then(|node| node.data_mut().as_element_mut())
    {
        element.mark_media_audio_source_created();
    }
    true
}
