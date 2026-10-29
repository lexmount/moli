use super::{NodeIteratorSnapshot, TreeWalkerSnapshot};
use crate::native_bridge::{JsContextHost, runtime_ptr_from_object};

pub(super) struct TraversalSnapshot<S> {
    pub(super) runtime_ptr: *mut JsContextHost,
    pub(super) id: u32,
    pub(super) state: S,
}

pub(super) fn node_iterator_snapshot_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<TraversalSnapshot<NodeIteratorSnapshot>> {
    let (runtime_ptr, id) = traversal_identity(scope, object)?;
    let state = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .node_iterator_snapshot(scope, id)?;
    Some(TraversalSnapshot {
        runtime_ptr,
        id,
        state,
    })
}

pub(super) fn tree_walker_snapshot_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<TraversalSnapshot<TreeWalkerSnapshot>> {
    let (runtime_ptr, id) = traversal_identity(scope, object)?;
    let state = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .tree_walker_snapshot(scope, id)?;
    Some(TraversalSnapshot {
        runtime_ptr,
        id,
        state,
    })
}

pub(super) fn traversal_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, u32)> {
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    let runtime_ptr = runtime_ptr_from_object(scope, object).ok()?;
    let value = object.get_internal_field(scope, 1)?;
    let value = v8::Local::<v8::Value>::try_from(value).ok()?;
    let number = value.number_value(scope)?;
    if !number.is_finite() || number < 1.0 || number.fract() != 0.0 {
        return None;
    }
    Some((runtime_ptr, number as u32))
}
