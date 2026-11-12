use super::super::filters::{
    PreparedTraversalFilter, TraversalFilterResult, traversal_filter_result,
};
use super::shared::child_handles;
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

pub(in crate::native_bridge::traversal) fn first_accepted_descendant(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    what_to_show: u32,
    filter: Option<&PreparedTraversalFilter>,
) -> Result<Option<DomHandle>, ()> {
    for child in child_handles(runtime_ptr, node) {
        match traversal_filter_result(scope, runtime_ptr, filter, child, what_to_show) {
            TraversalFilterResult::Accept => return Ok(Some(child)),
            TraversalFilterResult::Skip => {
                if let Some(nested) =
                    first_accepted_descendant(scope, runtime_ptr, child, what_to_show, filter)?
                {
                    return Ok(Some(nested));
                }
            }
            TraversalFilterResult::Reject | TraversalFilterResult::Other => {}
            TraversalFilterResult::Exception => return Err(()),
        }
    }
    Ok(None)
}

pub(in crate::native_bridge::traversal) fn last_accepted_descendant(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    what_to_show: u32,
    filter: Option<&PreparedTraversalFilter>,
) -> Result<Option<DomHandle>, ()> {
    let mut children = child_handles(runtime_ptr, node);
    children.reverse();
    for child in children {
        match traversal_filter_result(scope, runtime_ptr, filter, child, what_to_show) {
            TraversalFilterResult::Accept => return Ok(Some(child)),
            TraversalFilterResult::Skip => {
                if let Some(nested) =
                    last_accepted_descendant(scope, runtime_ptr, child, what_to_show, filter)?
                {
                    return Ok(Some(nested));
                }
            }
            TraversalFilterResult::Reject | TraversalFilterResult::Other => {}
            TraversalFilterResult::Exception => return Err(()),
        }
    }
    Ok(None)
}

pub(in crate::native_bridge::traversal) fn first_accepted_descendant_through_non_reject(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    what_to_show: u32,
    filter: Option<&PreparedTraversalFilter>,
) -> Result<Option<DomHandle>, ()> {
    for child in child_handles(runtime_ptr, node) {
        match traversal_filter_result(scope, runtime_ptr, filter, child, what_to_show) {
            TraversalFilterResult::Accept => return Ok(Some(child)),
            TraversalFilterResult::Reject => {}
            TraversalFilterResult::Skip | TraversalFilterResult::Other => {
                if let Some(nested) = first_accepted_descendant_through_non_reject(
                    scope,
                    runtime_ptr,
                    child,
                    what_to_show,
                    filter,
                )? {
                    return Ok(Some(nested));
                }
            }
            TraversalFilterResult::Exception => return Err(()),
        }
    }
    Ok(None)
}

pub(in crate::native_bridge::traversal) fn last_accepted_descendant_through_non_reject(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    node: DomHandle,
    what_to_show: u32,
    filter: Option<&PreparedTraversalFilter>,
) -> Result<Option<DomHandle>, ()> {
    let mut children = child_handles(runtime_ptr, node);
    children.reverse();
    for child in children {
        match traversal_filter_result(scope, runtime_ptr, filter, child, what_to_show) {
            TraversalFilterResult::Accept => return Ok(Some(child)),
            TraversalFilterResult::Reject => {}
            TraversalFilterResult::Skip | TraversalFilterResult::Other => {
                if let Some(nested) = last_accepted_descendant_through_non_reject(
                    scope,
                    runtime_ptr,
                    child,
                    what_to_show,
                    filter,
                )? {
                    return Ok(Some(nested));
                }
            }
            TraversalFilterResult::Exception => return Err(()),
        }
    }
    Ok(None)
}
