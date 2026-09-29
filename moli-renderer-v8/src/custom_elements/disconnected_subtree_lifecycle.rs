use super::super::{document_runtime::DomHandle, native_bridge::JsContextHost};
use super::connected_lifecycle::enqueue_disconnected_callback;
use super::traversal::collect_shadow_including_subtree_handles;

pub(crate) fn enqueue_disconnected_callbacks_for_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    root: DomHandle,
) {
    if unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent() {
        return;
    }
    let mut handles = Vec::new();
    collect_shadow_including_subtree_handles(host_ptr, root, &mut handles);
    for handle in handles {
        enqueue_disconnected_callback(scope, host_ptr, handle);
    }
}
