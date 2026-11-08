use crate::dom::native::NodeType;
use crate::native_bridge::NativeNodeReference;

pub(in crate::context_bootstrap) fn range_node_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u16> {
    match NativeNodeReference::from_object(scope, object)? {
        NativeNodeReference::Attr(_) => Some(2),
        NativeNodeReference::Tree { node, .. } => {
            let runtime = unsafe { &*node.runtime_ptr };
            Some(runtime.dom_host().node(node.handle)?.node_type() as u16)
        }
    }
}

pub(in crate::context_bootstrap) fn range_node_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let NativeNodeReference::Tree { object, node } =
        NativeNodeReference::from_object(scope, object)?
    else {
        return None;
    };
    let parent = unsafe { &*node.runtime_ptr }
        .dom_host()
        .node(node.handle)?
        .parent_node()?;
    unsafe { &mut *node.runtime_ptr }
        .native_bridge_mut()
        .wrap_handle_for_receiver(scope, node.runtime_ptr, object, parent)
}

pub(in crate::context_bootstrap) fn range_validate_boundary_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    container: v8::Local<'s, v8::Object>,
    offset: u32,
) -> bool {
    range_node_length(scope, container).is_some_and(|length| offset <= length)
}

pub(in crate::context_bootstrap) fn range_node_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    match NativeNodeReference::from_object(scope, node)? {
        NativeNodeReference::Attr(_) => Some(0),
        NativeNodeReference::Tree { node, .. } => {
            let runtime = unsafe { &*node.runtime_ptr };
            let native = runtime.dom_host().node(node.handle)?;
            let length = match native.node_type() {
                NodeType::Text
                | NodeType::CDataSection
                | NodeType::ProcessingInstruction
                | NodeType::Comment => runtime.character_data_utf16_units(node.handle)?.len(),
                _ => runtime.dom_host().child_handles(node.handle).count(),
            };
            u32::try_from(length).ok()
        }
    }
}

pub(in crate::context_bootstrap) fn child_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    let parent = NativeNodeReference::from_object(scope, parent)?.tree_node()?;
    let child = NativeNodeReference::from_object(scope, child)?.tree_node()?;
    if parent.runtime_ptr != child.runtime_ptr {
        return None;
    }
    unsafe { &*parent.runtime_ptr }
        .dom_host()
        .child_index(parent.handle, child.handle)
        .and_then(|index| u32::try_from(index).ok())
}
