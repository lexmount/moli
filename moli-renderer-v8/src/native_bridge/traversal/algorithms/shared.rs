use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

pub(super) fn child_handles(runtime_ptr: *mut PageScriptHost, node: DomHandle) -> Vec<DomHandle> {
    unsafe { &*runtime_ptr }
        .dom_host()
        .child_handles(node)
        .collect()
}

pub(super) fn node_is_inside_root(
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    root: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let mut current = node;
    loop {
        if current == root {
            return true;
        }
        let Some(parent) = runtime.dom_host().dom().parent_node(current) else {
            return false;
        };
        current = parent;
    }
}

pub(super) fn next_preorder_node(
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    root: DomHandle,
) -> Option<DomHandle> {
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host().dom();
    if let Some(child) = dom.first_child(node) {
        return Some(child);
    }

    let mut current = node;
    loop {
        if current == root {
            return None;
        }
        if let Some(sibling) = dom.next_sibling(current) {
            return Some(sibling);
        }
        current = dom.parent_node(current)?;
    }
}

pub(super) fn previous_preorder_node(
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    root: DomHandle,
) -> Option<DomHandle> {
    if node == root {
        return None;
    }

    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host().dom();
    if let Some(sibling) = dom.previous_sibling(node) {
        return Some(last_preorder_descendant(runtime_ptr, sibling));
    }
    dom.parent_node(node)
}

fn last_preorder_descendant(runtime_ptr: *mut PageScriptHost, node: DomHandle) -> DomHandle {
    let runtime = unsafe { &*runtime_ptr };
    let mut current = node;
    while let Some(child) = runtime.dom_host().dom().last_child(current) {
        current = child;
    }
    current
}
