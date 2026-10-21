use super::*;
use crate::native_bridge::NativeNodeReference;
use crate::range_boundary::RangeBoundaryPoint as NativeRangeBoundaryPoint;
use crate::util::{object_chain_contains, walk_object_chain};

pub(in crate::context_bootstrap) fn native_boundary_point_from_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
) -> Option<NativeRangeBoundaryPoint> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    NativeRangeBoundaryPoint::new_for_offset_validation(
        unsafe { &*host_ptr }.dom_host(),
        node_handle_for_tree_op(scope, node)?,
        offset,
    )
}

pub(in crate::context_bootstrap) fn native_boundary_point_from_range_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
) -> Option<NativeRangeBoundaryPoint> {
    if let Some(point) = native_range_boundary_point(scope, range, side) {
        return Some(point);
    }
    let container = range_boundary_container_object(scope, range, side)?;
    let offset = range_boundary_offset(scope, range, side) as u32;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    NativeRangeBoundaryPoint::new(
        unsafe { &*host_ptr }.dom_host(),
        node_handle_for_tree_op(scope, container)?,
        offset,
    )
}

pub(in crate::context_bootstrap) fn root_handle(
    scope: &mut v8::PinScope<'_, '_>,
    mut handle: DomHandle,
) -> Option<DomHandle> {
    while let Some(parent) = parent_handle(scope, handle) {
        handle = parent;
    }
    Some(handle)
}

pub(in crate::context_bootstrap) fn native_boundary_points_share_root(
    scope: &mut v8::PinScope<'_, '_>,
    a: NativeRangeBoundaryPoint,
    b: NativeRangeBoundaryPoint,
) -> bool {
    root_handle(scope, a.container()) == root_handle(scope, b.container())
}

pub(in crate::context_bootstrap) fn native_boundary_point_is_valid(
    scope: &mut v8::PinScope<'_, '_>,
    mut point: NativeRangeBoundaryPoint,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let dom_host = unsafe { &*host_ptr }.dom_host();
    let Some(offset) = point.offset(dom_host) else {
        return false;
    };
    range_node_length_handle(scope, point.container()).is_some_and(|length| offset <= length)
}

pub(in crate::context_bootstrap) fn native_boundary_point_is_doctype(
    scope: &mut v8::PinScope<'_, '_>,
    point: NativeRangeBoundaryPoint,
) -> bool {
    node_type_for_handle(scope, point.container()) == Some(NodeType::DocumentType)
}

pub(in crate::context_bootstrap) fn native_boundary_point_order(
    scope: &mut v8::PinScope<'_, '_>,
    mut a: NativeRangeBoundaryPoint,
    mut b: NativeRangeBoundaryPoint,
) -> Option<std::cmp::Ordering> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let dom_host = unsafe { &*host_ptr }.dom_host();
    let a_offset = a.offset(dom_host)?;
    let b_offset = b.offset(dom_host)?;
    point_order_handles(scope, a.container(), a_offset, b.container(), b_offset)
}

fn handle_chain(scope: &mut v8::PinScope<'_, '_>, mut handle: DomHandle) -> Vec<DomHandle> {
    let mut chain = vec![handle];
    while let Some(parent) = parent_handle(scope, handle) {
        chain.push(parent);
        handle = parent;
    }
    chain
}

pub(in crate::context_bootstrap::range_algorithms) fn common_ancestor_handle(
    scope: &mut v8::PinScope<'_, '_>,
    start: DomHandle,
    end: DomHandle,
) -> Option<DomHandle> {
    if start == end {
        return Some(start);
    }

    let start_chain = handle_chain(scope, start);
    let end_chain = handle_chain(scope, end);
    let mut si = start_chain.len();
    let mut ei = end_chain.len();
    let mut lca = None;
    while si > 0 && ei > 0 && start_chain[si - 1] == end_chain[ei - 1] {
        lca = Some(start_chain[si - 1]);
        si -= 1;
        ei -= 1;
    }
    if lca.is_some() {
        return lca;
    }
    start_chain
        .into_iter()
        .find(|node| end_chain.contains(node))
}

pub(in crate::context_bootstrap) fn point_order_handles(
    scope: &mut v8::PinScope<'_, '_>,
    a_container: DomHandle,
    a_offset: u32,
    b_container: DomHandle,
    b_offset: u32,
) -> Option<std::cmp::Ordering> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    crate::range_boundary::point_order_in_dom(
        unsafe { &*host_ptr }.dom_host(),
        a_container,
        a_offset,
        b_container,
        b_offset,
    )
}

/// Compare native boundary points without evaluating author-visible tree properties.
pub(in crate::context_bootstrap) fn point_order<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    a_container: v8::Local<'s, v8::Object>,
    a_offset: u32,
    b_container: v8::Local<'s, v8::Object>,
    b_offset: u32,
) -> Option<std::cmp::Ordering> {
    let a = NativeNodeReference::from_object(scope, a_container)?;
    let b = NativeNodeReference::from_object(scope, b_container)?;
    if a.is_same(scope, &b) {
        return Some(a_offset.cmp(&b_offset));
    }
    let a = a.tree_node()?;
    let b = b.tree_node()?;
    if a.runtime_ptr != b.runtime_ptr {
        return None;
    }
    crate::range_boundary::point_order_in_dom(
        unsafe { &*a.runtime_ptr }.dom_host(),
        a.handle,
        a_offset,
        b.handle,
        b_offset,
    )
}

pub(in crate::context_bootstrap) fn range_compare_point_internal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
    offset: u32,
) -> i32 {
    if let Some(boundaries) = native_range_boundary_handles(scope, range)
        && let Some(node_handle) = node_handle_for_tree_op(scope, node)
        && let Some(result) = range_compare_point_handles(
            scope,
            node_handle,
            offset,
            boundaries.start.container,
            boundaries.start.offset,
            boundaries.end.container,
            boundaries.end.offset,
        )
    {
        return result;
    }

    let Some(start) = range_boundary_container_object(scope, range, RangeBoundarySide::Start)
    else {
        return 0;
    };
    let Some(end) = range_boundary_container_object(scope, range, RangeBoundarySide::End) else {
        return 0;
    };
    let start_offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32;
    let end_offset = range_boundary_offset(scope, range, RangeBoundarySide::End) as u32;
    let start_order = point_order(scope, node, offset, start, start_offset);
    let end_order = point_order(scope, node, offset, end, end_offset);
    if start_order == Some(std::cmp::Ordering::Less) {
        -1
    } else if end_order == Some(std::cmp::Ordering::Greater) {
        1
    } else {
        0
    }
}

pub(in crate::context_bootstrap) fn range_intersects_node_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    let boundaries = native_range_boundary_handles(scope, range)?;
    let node_handle = node_handle_for_tree_op(scope, node)?;
    if root_handle(scope, node_handle) != root_handle(scope, boundaries.start.container) {
        return Some(false);
    }
    let Some(parent) = parent_handle(scope, node_handle) else {
        return Some(true);
    };
    let index = child_index_handle(scope, parent, node_handle)?;
    let start_cmp = range_compare_point_handles(
        scope,
        parent,
        index,
        boundaries.start.container,
        boundaries.start.offset,
        boundaries.end.container,
        boundaries.end.offset,
    )?;
    let end_cmp = range_compare_point_handles(
        scope,
        parent,
        index + 1,
        boundaries.start.container,
        boundaries.start.offset,
        boundaries.end.container,
        boundaries.end.offset,
    )?;
    let node_starts_before_range_end = point_order_handles(
        scope,
        parent,
        index,
        boundaries.end.container,
        boundaries.end.offset,
    ) == Some(std::cmp::Ordering::Less);
    let node_ends_after_range_start = point_order_handles(
        scope,
        parent,
        index + 1,
        boundaries.start.container,
        boundaries.start.offset,
    ) == Some(std::cmp::Ordering::Greater);
    Some(
        start_cmp != 1
            && end_cmp != -1
            && node_starts_before_range_end
            && node_ends_after_range_start,
    )
}

fn range_compare_point_handles(
    scope: &mut v8::PinScope<'_, '_>,
    node_handle: DomHandle,
    offset: u32,
    start_container: DomHandle,
    start_offset: u32,
    end_container: DomHandle,
    end_offset: u32,
) -> Option<i32> {
    let start_order =
        point_order_handles(scope, node_handle, offset, start_container, start_offset);
    let end_order = point_order_handles(scope, node_handle, offset, end_container, end_offset);
    if start_order == Some(std::cmp::Ordering::Less) {
        Some(-1)
    } else if end_order == Some(std::cmp::Ordering::Greater) {
        Some(1)
    } else if start_order.is_some() && end_order.is_some() {
        Some(0)
    } else {
        None
    }
}

pub(in crate::context_bootstrap) fn range_common_ancestor_container<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(boundaries) = native_range_boundary_handles(scope, range) {
        let handle =
            common_ancestor_handle(scope, boundaries.start.container, boundaries.end.container)?;
        return node_wrapper_for_handle_prefer_paired(scope, handle);
    }

    let start = range_boundary_container_object(scope, range, RangeBoundarySide::Start)?;
    let end = range_boundary_container_object(scope, range, RangeBoundarySide::End)?;
    if start.strict_equals(end.into()) {
        return Some(start);
    }
    let start_chain = walk_object_chain(scope, start, "parentNode");
    let end_chain = walk_object_chain(scope, end, "parentNode");
    // O(d_start + d_end): trim equal suffix from the root side; the last
    // surviving common element is the LCA. Falls back to linear search if the
    // chains share no root, matching the previous behavior's None.
    let mut si = start_chain.len();
    let mut ei = end_chain.len();
    let mut lca: Option<v8::Local<'s, v8::Object>> = None;
    while si > 0 && ei > 0 && start_chain[si - 1].strict_equals(end_chain[ei - 1].into()) {
        lca = Some(start_chain[si - 1]);
        si -= 1;
        ei -= 1;
    }
    if lca.is_some() {
        return lca;
    }
    // No shared root: preserve previous semantics by reporting no common
    // ancestor (callers handle this as detached / cross-tree).
    start_chain
        .into_iter()
        .find(|node| object_chain_contains(&end_chain, *node))
}
