use super::*;

pub(in crate::context_bootstrap) fn node_owner_document_or_self<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let node_type = object_number_property(scope, node, "nodeType")? as u32;
    if node_type == 9 {
        return Some(node);
    }
    object_property_as_object(scope, node, "ownerDocument")
}
