use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.comparePoint")]
struct RangeComparePointArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    node: v8::Local<'s, v8::Object>,
    #[webidl(required)]
    offset: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.isPointInRange")]
struct RangeIsPointInRangeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    node: v8::Local<'s, v8::Object>,
    #[webidl(required)]
    offset: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.compareBoundaryPoints")]
struct RangeCompareBoundaryPointsArgs<'s> {
    #[webidl(required)]
    how: u16,
    #[webidl(required, interface = web_api_interfaces::Range)]
    source_range: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.intersectsNode")]
struct RangeIntersectsNodeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    node: v8::Local<'s, v8::Object>,
}

pub(super) fn range_compare_point_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeComparePointArgs<'s>>(scope, &args) else {
        return;
    };
    let node = parsed.node;
    let offset = parsed.offset;
    // Spec Range.comparePoint(node, offset):
    //   1. If node's root != range's root, throw WrongDocumentError.
    //   2. If node is a DocumentType, throw InvalidNodeTypeError.
    //   3. If offset > node.length, throw IndexSizeError.
    //   4. Otherwise compute the relative order.
    let Some(point) = native_boundary_point_from_node(scope, node, offset) else {
        throw_named_dom_exception(
            scope,
            "WrongDocumentError",
            "The node is in a different document than the range.",
        );
        return;
    };
    let Some(start_point) =
        native_boundary_point_from_range_boundary(scope, args.this(), RangeBoundarySide::Start)
    else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    if !native_boundary_points_share_root(scope, point, start_point) {
        throw_named_dom_exception(
            scope,
            "WrongDocumentError",
            "The node is in a different document than the range.",
        );
        return;
    }
    if native_boundary_point_is_doctype(scope, point) {
        throw_named_dom_exception(scope, "InvalidNodeTypeError", "The node is a DocumentType.");
        return;
    }
    if !native_boundary_point_is_valid(scope, point) {
        throw_named_dom_exception(
            scope,
            "IndexSizeError",
            "Index or size is negative or greater than the allowed amount.",
        );
        return;
    }
    let Some(end_point) =
        native_boundary_point_from_range_boundary(scope, args.this(), RangeBoundarySide::End)
    else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    let start_order = native_boundary_point_order(scope, point, start_point);
    let end_order = native_boundary_point_order(scope, point, end_point);
    let result = if start_order == Some(std::cmp::Ordering::Less) {
        -1
    } else if end_order == Some(std::cmp::Ordering::Greater) {
        1
    } else {
        0
    };
    rv.set(v8::Integer::new(scope, result).into());
}

pub(super) fn range_is_point_in_range_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeIsPointInRangeArgs<'s>>(scope, &args) else {
        return;
    };
    let node = parsed.node;
    let offset = parsed.offset;
    // Spec Range.isPointInRange(node, offset):
    //   1. If node's root != range's root, return false (no throw).
    //   2. If node is a DocumentType, throw InvalidNodeTypeError.
    //   3. If offset > node.length, throw IndexSizeError.
    //   4. Otherwise compute containment.
    let Some(point) = native_boundary_point_from_node(scope, node, offset) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let Some(start_point) =
        native_boundary_point_from_range_boundary(scope, args.this(), RangeBoundarySide::Start)
    else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    if !native_boundary_points_share_root(scope, point, start_point) {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    }
    if native_boundary_point_is_doctype(scope, point) {
        throw_named_dom_exception(scope, "InvalidNodeTypeError", "The node is a DocumentType.");
        return;
    }
    if !native_boundary_point_is_valid(scope, point) {
        throw_named_dom_exception(
            scope,
            "IndexSizeError",
            "Index or size is negative or greater than the allowed amount.",
        );
        return;
    }
    let Some(end_point) =
        native_boundary_point_from_range_boundary(scope, args.this(), RangeBoundarySide::End)
    else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let start_order = native_boundary_point_order(scope, point, start_point);
    let end_order = native_boundary_point_order(scope, point, end_point);
    let inside = start_order != Some(std::cmp::Ordering::Less)
        && end_order != Some(std::cmp::Ordering::Greater);
    rv.set(v8::Boolean::new(scope, inside).into());
}

pub(super) fn range_intersects_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeIntersectsNodeArgs<'s>>(scope, &args) else {
        return;
    };
    let node = parsed.node;
    if let Some(intersects) = range_intersects_node_native(scope, args.this(), node) {
        rv.set(v8::Boolean::new(scope, intersects).into());
        return;
    }
    let Some(start) = range_boundary_container_object(scope, args.this(), RangeBoundarySide::Start)
    else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let Some(end) = range_boundary_container_object(scope, args.this(), RangeBoundarySide::End)
    else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let Some(start_handle) = native_bridge::callback_value_dom_handle(scope, start.into()) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let Some(node_handle) = native_bridge::callback_value_dom_handle(scope, node.into()) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    if root_handle(scope, node_handle) != root_handle(scope, start_handle) {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    }

    let Some(parent) = range_node_parent(scope, node) else {
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    };
    let Some(index) = child_index(scope, parent, node) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let start_cmp = range_compare_point_internal(scope, args.this(), parent, index);
    let end_cmp = range_compare_point_internal(scope, args.this(), parent, index + 1);
    let start_offset = range_boundary_offset(scope, args.this(), RangeBoundarySide::Start) as u32;
    let end_offset = range_boundary_offset(scope, args.this(), RangeBoundarySide::End) as u32;
    let node_starts_before_range_end =
        point_order(scope, parent, index, end, end_offset) == Some(std::cmp::Ordering::Less);
    let node_ends_after_range_start = point_order(scope, parent, index + 1, start, start_offset)
        == Some(std::cmp::Ordering::Greater);
    rv.set(
        v8::Boolean::new(
            scope,
            start_cmp != 1
                && end_cmp != -1
                && node_starts_before_range_end
                && node_ends_after_range_start,
        )
        .into(),
    );
}

pub(super) fn range_compare_boundary_points_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeCompareBoundaryPointsArgs<'s>>(scope, &args)
    else {
        return;
    };
    let how = parsed.how;
    let other = parsed.source_range;
    if how > 3 {
        throw_named_dom_exception(
            scope,
            "NotSupportedError",
            "The operation is not supported.",
        );
        return;
    }
    let (this_side, other_side) = match how {
        0 => (RangeBoundarySide::Start, RangeBoundarySide::Start),
        1 => (RangeBoundarySide::End, RangeBoundarySide::Start),
        2 => (RangeBoundarySide::End, RangeBoundarySide::End),
        3 => (RangeBoundarySide::Start, RangeBoundarySide::End),
        _ => unreachable!(),
    };
    let Some(this_start_point) =
        native_boundary_point_from_range_boundary(scope, args.this(), RangeBoundarySide::Start)
    else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    let Some(other_start_point) =
        native_boundary_point_from_range_boundary(scope, other, RangeBoundarySide::Start)
    else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    if !native_boundary_points_share_root(scope, this_start_point, other_start_point) {
        throw_named_dom_exception(
            scope,
            "WrongDocumentError",
            "The object is in the wrong document.",
        );
        return;
    }
    let Some(this_point) = native_boundary_point_from_range_boundary(scope, args.this(), this_side)
    else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    let Some(other_point) = native_boundary_point_from_range_boundary(scope, other, other_side)
    else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    let Some(order) = native_boundary_point_order(scope, this_point, other_point) else {
        throw_named_dom_exception(
            scope,
            "WrongDocumentError",
            "The object is in the wrong document.",
        );
        return;
    };
    let result = match order {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    rv.set(v8::Integer::new(scope, result).into());
}
