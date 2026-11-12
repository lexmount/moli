use super::super::{document_runtime::DomHandle, native_bridge::PageScriptHost};
use super::connected_lifecycle::enqueue_connected_callback;
use super::form_lifecycle::{
    enqueue_form_association_callback_if_needed, enqueue_form_disabled_callback_if_needed,
};
use super::is_shadow_including_rooted_in_document;
use super::traversal::collect_shadow_including_subtree_handles;
use std::collections::HashSet;

pub(crate) fn enqueue_connected_and_form_callbacks_for_already_upgraded_subtrees(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    roots: &[DomHandle],
) -> bool {
    let host = unsafe { &*host_ptr };
    if roots.is_empty() || host.custom_elements_subtree_lifecycle_quiescent() {
        return false;
    }

    let mut visited_roots = HashSet::new();
    let mut enqueued_handles = HashSet::new();
    let mut connected_handles = Vec::new();
    let mut enqueued = false;
    for &root in roots {
        if !visited_roots.insert(root) {
            continue;
        }
        let host = unsafe { &*host_ptr };
        if !is_shadow_including_rooted_in_document(host.dom_host(), root) {
            continue;
        }
        let mut handles = Vec::new();
        collect_shadow_including_subtree_handles(host_ptr, root, &mut handles);
        for handle in handles {
            if !enqueued_handles.insert(handle) {
                continue;
            }
            connected_handles.push(handle);
            if enqueue_connected_callback(scope, host_ptr, handle) {
                enqueued = true;
            }
        }
    }
    for handle in connected_handles {
        if enqueue_form_association_callback_if_needed(scope, host_ptr, handle) {
            enqueued = true;
        }
        if enqueue_form_disabled_callback_if_needed(scope, host_ptr, handle) {
            enqueued = true;
        }
    }
    enqueued
}
