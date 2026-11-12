use super::super::{
    TreeWalkerSnapshot,
    filters::{PreparedTraversalFilter, TraversalFilterResult, traversal_filter_result},
};
use super::descendants::{
    first_accepted_descendant, first_accepted_descendant_through_non_reject,
    last_accepted_descendant_through_non_reject,
};
use super::shared::child_handles;
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

pub(in crate::native_bridge::traversal) fn tree_walker_parent_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    state: &TreeWalkerSnapshot,
) -> Result<Option<DomHandle>, ()> {
    if state.current_node == state.root {
        return Ok(None);
    }
    let runtime = unsafe { &*runtime_ptr };
    let mut current = runtime.dom_host().dom().parent_node(state.current_node);
    while let Some(node) = current {
        let result = traversal_filter_result(
            scope,
            runtime_ptr,
            state.filter.as_deref(),
            node,
            state.what_to_show,
        );
        match result {
            TraversalFilterResult::Accept => return Ok(Some(node)),
            TraversalFilterResult::Exception => return Err(()),
            TraversalFilterResult::Reject
            | TraversalFilterResult::Skip
            | TraversalFilterResult::Other => {}
        }
        if node == state.root {
            break;
        }
        current = runtime.dom_host().dom().parent_node(node);
    }
    Ok(None)
}

pub(in crate::native_bridge::traversal) fn tree_walker_next_sibling(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    state: &TreeWalkerSnapshot,
) -> Result<Option<DomHandle>, ()> {
    if state.current_node == state.root {
        return Ok(None);
    }
    let runtime = unsafe { &*runtime_ptr };
    let mut current = state.current_node;
    loop {
        let mut sibling = runtime.dom_host().dom().next_sibling(current);
        while let Some(node) = sibling {
            let result = traversal_filter_result(
                scope,
                runtime_ptr,
                state.filter.as_deref(),
                node,
                state.what_to_show,
            );
            match result {
                TraversalFilterResult::Accept => return Ok(Some(node)),
                TraversalFilterResult::Skip | TraversalFilterResult::Other => {
                    if let Some(nested) = first_accepted_descendant_through_non_reject(
                        scope,
                        runtime_ptr,
                        node,
                        state.what_to_show,
                        state.filter.as_deref(),
                    )? {
                        return Ok(Some(nested));
                    }
                }
                TraversalFilterResult::Reject => {}
                TraversalFilterResult::Exception => return Err(()),
            }
            sibling = runtime.dom_host().dom().next_sibling(node);
        }

        let Some(parent) = runtime.dom_host().dom().parent_node(current) else {
            return Ok(None);
        };
        if parent == state.root {
            return Ok(None);
        }
        match traversal_filter_result(
            scope,
            runtime_ptr,
            state.filter.as_deref(),
            parent,
            state.what_to_show,
        ) {
            TraversalFilterResult::Exception => return Err(()),
            TraversalFilterResult::Skip | TraversalFilterResult::Other => {}
            TraversalFilterResult::Accept | TraversalFilterResult::Reject => return Ok(None),
        }
        current = parent;
    }
}

pub(in crate::native_bridge::traversal) fn tree_walker_previous_sibling(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    state: &TreeWalkerSnapshot,
) -> Result<Option<DomHandle>, ()> {
    if state.current_node == state.root {
        return Ok(None);
    }
    let runtime = unsafe { &*runtime_ptr };
    let mut current = state.current_node;
    loop {
        let mut sibling = runtime.dom_host().dom().previous_sibling(current);
        while let Some(node) = sibling {
            let result = traversal_filter_result(
                scope,
                runtime_ptr,
                state.filter.as_deref(),
                node,
                state.what_to_show,
            );
            match result {
                TraversalFilterResult::Accept => return Ok(Some(node)),
                TraversalFilterResult::Skip | TraversalFilterResult::Other => {
                    if let Some(nested) = last_accepted_descendant_through_non_reject(
                        scope,
                        runtime_ptr,
                        node,
                        state.what_to_show,
                        state.filter.as_deref(),
                    )? {
                        return Ok(Some(nested));
                    }
                }
                TraversalFilterResult::Reject => {}
                TraversalFilterResult::Exception => return Err(()),
            }
            sibling = runtime.dom_host().dom().previous_sibling(node);
        }

        let Some(parent) = runtime.dom_host().dom().parent_node(current) else {
            return Ok(None);
        };
        if parent == state.root {
            return Ok(None);
        }
        match traversal_filter_result(
            scope,
            runtime_ptr,
            state.filter.as_deref(),
            parent,
            state.what_to_show,
        ) {
            TraversalFilterResult::Exception => return Err(()),
            TraversalFilterResult::Skip | TraversalFilterResult::Other => {}
            TraversalFilterResult::Accept | TraversalFilterResult::Reject => return Ok(None),
        }
        current = parent;
    }
}

pub(in crate::native_bridge::traversal) fn tree_walker_next_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    state: &TreeWalkerSnapshot,
) -> Result<Option<DomHandle>, ()> {
    if let Some(descendant) = first_accepted_descendant(
        scope,
        runtime_ptr,
        state.current_node,
        state.what_to_show,
        state.filter.as_deref(),
    )? {
        return Ok(Some(descendant));
    }

    let mut current = state.current_node;
    loop {
        if current == state.root {
            return Ok(None);
        }
        let snapshot = TreeWalkerSnapshot {
            root: state.root,
            what_to_show: state.what_to_show,
            filter: state.filter.clone(),
            current_node: current,
        };
        if let Some(sibling) = tree_walker_next_sibling(scope, runtime_ptr, &snapshot)? {
            return Ok(Some(sibling));
        }
        current = unsafe { &*runtime_ptr }
            .dom_host()
            .dom()
            .parent_node(current)
            .unwrap_or(state.root);
    }
}

pub(in crate::native_bridge::traversal) fn tree_walker_previous_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    state: &TreeWalkerSnapshot,
) -> Result<Option<DomHandle>, ()> {
    if state.current_node == state.root {
        return Ok(None);
    }
    let runtime = unsafe { &*runtime_ptr };
    let mut current = state.current_node;
    loop {
        let mut sibling = runtime.dom_host().dom().previous_sibling(current);
        while let Some(node) = sibling {
            if let Some(accepted) = last_accepted_in_tree_walker_subtree(
                scope,
                runtime_ptr,
                node,
                state.what_to_show,
                state.filter.as_deref(),
            )? {
                return Ok(Some(accepted));
            }
            sibling = runtime.dom_host().dom().previous_sibling(node);
        }

        let Some(parent) = runtime.dom_host().dom().parent_node(current) else {
            return Ok(None);
        };
        current = parent;
        let result = traversal_filter_result(
            scope,
            runtime_ptr,
            state.filter.as_deref(),
            current,
            state.what_to_show,
        );
        match result {
            TraversalFilterResult::Accept => return Ok(Some(current)),
            TraversalFilterResult::Exception => return Err(()),
            TraversalFilterResult::Reject
            | TraversalFilterResult::Skip
            | TraversalFilterResult::Other => {}
        }
        if current == state.root {
            return Ok(None);
        }
    }
}

fn last_accepted_in_tree_walker_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    what_to_show: u32,
    filter: Option<&PreparedTraversalFilter>,
) -> Result<Option<DomHandle>, ()> {
    let result = traversal_filter_result(scope, runtime_ptr, filter, node, what_to_show);
    if result == TraversalFilterResult::Exception {
        return Err(());
    }
    if result != TraversalFilterResult::Reject {
        for child in child_handles(runtime_ptr, node).into_iter().rev() {
            if let Some(accepted) = last_accepted_in_tree_walker_subtree(
                scope,
                runtime_ptr,
                child,
                what_to_show,
                filter,
            )? {
                return Ok(Some(accepted));
            }
        }
    }
    if result == TraversalFilterResult::Accept {
        return Ok(Some(node));
    }
    Ok(None)
}
