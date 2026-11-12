use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

pub(super) fn shadow_including_subtree_handles(
    host_ptr: *mut PageScriptHost,
    root: DomHandle,
) -> Vec<DomHandle> {
    let mut handles = Vec::new();
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        handles.push(handle);
        let children = shadow_including_child_handles(host_ptr, handle);
        stack.extend(children.into_iter().rev());
    }
    handles
}

pub(super) fn collect_shadow_including_subtree_handles(
    host_ptr: *mut PageScriptHost,
    root: DomHandle,
    handles: &mut Vec<DomHandle>,
) {
    handles.extend(shadow_including_subtree_handles(host_ptr, root));
}

pub(super) fn shadow_including_child_handles(
    host_ptr: *mut PageScriptHost,
    root: DomHandle,
) -> Vec<DomHandle> {
    let dom_host = unsafe { &*host_ptr }.dom_host();
    let mut children = Vec::new();
    if let Some(shadow_root) = dom_host.shadow_root_handle(root) {
        children.push(shadow_root);
    }
    children.extend(dom_host.child_handles(root));
    children
}
