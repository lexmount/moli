use super::*;
use crate::util::new_null_prototype_object;

pub(in crate::native_bridge::document) fn new_map_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Map> {
    v8::Map::new(scope)
}

pub(in crate::native_bridge::document) fn new_detached_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    node_type: i32,
    node_name: impl Into<moli_dom::native::DomStringValue>,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = new_null_prototype_object(scope);
    let _ = state.set(
        scope,
        v8str(scope, "kind").into(),
        v8_string(scope, kind)?.into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "nodeType").into(),
        v8::Integer::new(scope, node_type).into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "nodeName").into(),
        crate::util::v8_string_from_dom_string_value(scope, &node_name.into())?.into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        v8::null(scope).into(),
    );
    Some(state)
}
