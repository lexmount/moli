use super::super::super::{detached_child_node_objects, detached_node_type};

pub(super) fn find_detached_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Object>,
    matches: &mut impl FnMut(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>) -> bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if detached_node_type(scope, node) == Some(1) && matches(scope, node) {
            return Some(node);
        }
        stack.extend(detached_child_node_objects(scope, node).into_iter().rev());
    }
    None
}
