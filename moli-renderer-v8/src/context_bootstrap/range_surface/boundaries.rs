use super::super::selection::{
    SelectionRangeUpdateState, selection_clear, selection_composed_boundary_order,
    selection_range_update_state, selection_sync_associated_range,
};
use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.setStart")]
struct RangeSetStartArgs<'s> {
    #[webidl(with = range_set_start_node_arg)]
    container: v8::Local<'s, v8::Object>,
    #[webidl(required)]
    offset: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.setEnd")]
struct RangeSetEndArgs<'s> {
    #[webidl(with = range_set_end_node_arg)]
    container: v8::Local<'s, v8::Object>,
    #[webidl(required)]
    offset: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.selectNodeContents")]
struct RangeSelectNodeContentsArgs<'s> {
    #[webidl(with = range_select_node_contents_node_arg)]
    node: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.selectNode")]
struct RangeSelectNodeArgs<'s> {
    #[webidl(with = range_select_node_node_arg)]
    node: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range.collapse")]
struct RangeCollapseArgs {
    #[webidl(default = false)]
    to_start: bool,
}

fn range_set_start_node_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    webidl_node_arg(scope, args, index, "Range.setStart requires a Node")
}

fn range_set_end_node_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    webidl_node_arg(scope, args, index, "Range.setEnd requires a Node")
}

fn range_select_node_contents_node_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    webidl_node_arg(
        scope,
        args,
        index,
        "Range.selectNodeContents requires a Node",
    )
}

fn range_select_node_node_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    webidl_node_arg(scope, args, index, "Range.selectNode requires a Node")
}

pub(super) fn range_set_start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeSetStartArgs<'s>>(scope, &args) else {
        return;
    };
    let container = parsed.container;
    let offset = parsed.offset;
    // Spec Range.setStart(node, offset):
    //   1. If node is a doctype, throw InvalidNodeTypeError.
    //   2. If offset > node.length, throw IndexSizeError.
    if node_is_doctype(scope, container) {
        throw_named_dom_exception(
            scope,
            "InvalidNodeTypeError",
            "Range.setStart cannot target a DocumentType.",
        );
        return;
    }
    if !range_validate_boundary_point(scope, container, offset) {
        throw_named_dom_exception(
            scope,
            "IndexSizeError",
            "Index or size is negative or greater than the allowed amount.",
        );
        return;
    }
    range_set_boundary_and_sync_selection(
        scope,
        args.this(),
        RangeBoundarySide::Start,
        container,
        offset,
    );
    rv.set_undefined();
}

pub(super) fn range_set_end_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeSetEndArgs<'s>>(scope, &args) else {
        return;
    };
    let container = parsed.container;
    let offset = parsed.offset;
    if node_is_doctype(scope, container) {
        throw_named_dom_exception(
            scope,
            "InvalidNodeTypeError",
            "Range.setEnd cannot target a DocumentType.",
        );
        return;
    }
    if !range_validate_boundary_point(scope, container, offset) {
        throw_named_dom_exception(
            scope,
            "IndexSizeError",
            "Index or size is negative or greater than the allowed amount.",
        );
        return;
    }
    range_set_boundary_and_sync_selection(
        scope,
        args.this(),
        RangeBoundarySide::End,
        container,
        offset,
    );
    rv.set_undefined();
}

pub(super) fn range_select_node_contents_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeSelectNodeContentsArgs<'s>>(scope, &args) else {
        return;
    };
    let node = parsed.node;
    if node_is_doctype(scope, node) {
        throw_named_dom_exception(
            scope,
            "InvalidNodeTypeError",
            "Range.selectNodeContents cannot target a DocumentType.",
        );
        return;
    }
    let selection_update = selection_range_update_state(scope, args.this());
    let end_offset = range_node_length(scope, node).unwrap_or(0);
    set_range_boundary(scope, args.this(), RangeBoundarySide::Start, node, 0);
    set_range_boundary(scope, args.this(), RangeBoundarySide::End, node, end_offset);
    range_sync_selection_to_current_range(scope, selection_update, args.this());
    rv.set_undefined();
}

pub(super) fn range_collapse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeCollapseArgs>(scope, &args) else {
        return;
    };
    let to_start = parsed.to_start;
    let target_side = if to_start {
        RangeBoundarySide::Start
    } else {
        RangeBoundarySide::End
    };
    let target_container = range_boundary_container_object(scope, args.this(), target_side);
    let target_offset = range_boundary_offset(scope, args.this(), target_side) as u32;
    let Some(target_container) = target_container else {
        rv.set_undefined();
        return;
    };
    let selection_update = selection_range_update_state(scope, args.this());
    set_range_boundary(
        scope,
        args.this(),
        RangeBoundarySide::Start,
        target_container,
        target_offset,
    );
    set_range_boundary(
        scope,
        args.this(),
        RangeBoundarySide::End,
        target_container,
        target_offset,
    );
    range_sync_selection_to_current_range(scope, selection_update, args.this());
    rv.set_undefined();
}

pub(super) fn range_select_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RangeSelectNodeArgs<'s>>(scope, &args) else {
        return;
    };
    let node = parsed.node;
    let Some(parent) = object_property_as_object(scope, node, "parentNode") else {
        throw_named_dom_exception(
            scope,
            "InvalidNodeTypeError",
            "Range.selectNode requires a node with a parent.",
        );
        return;
    };
    let Some(index) = child_index(scope, parent, node) else {
        rv.set_undefined();
        return;
    };
    let selection_update = selection_range_update_state(scope, args.this());
    set_range_boundary(scope, args.this(), RangeBoundarySide::Start, parent, index);
    set_range_boundary(
        scope,
        args.this(),
        RangeBoundarySide::End,
        parent,
        index + 1,
    );
    range_sync_selection_to_current_range(scope, selection_update, args.this());
    rv.set_undefined();
}

pub(super) fn range_set_start_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    range_set_boundary_relative(scope, args, rv, true, false);
}

pub(super) fn range_set_start_after_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    range_set_boundary_relative(scope, args, rv, true, true);
}

pub(super) fn range_set_end_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    range_set_boundary_relative(scope, args, rv, false, false);
}

pub(super) fn range_set_end_after_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    range_set_boundary_relative(scope, args, rv, false, true);
}

pub(super) fn range_clone_range_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(document) =
        range_boundary_container_object(scope, args.this(), RangeBoundarySide::Start)
            .and_then(|node| node_owner_document_or_self(scope, node))
    else {
        rv.set_undefined();
        return;
    };
    let Some(clone) = new_range_for_document(scope, document) else {
        rv.set_undefined();
        return;
    };
    for side in [RangeBoundarySide::Start, RangeBoundarySide::End] {
        let Some(container) = range_boundary_container_object(scope, args.this(), side) else {
            continue;
        };
        let offset = range_boundary_offset(scope, args.this(), side) as u32;
        set_range_boundary(scope, clone, side, container, offset);
    }
    rv.set(clone.into());
}

/// True iff `node`'s nodeType is 10 (DocumentType). Used by Range boundary
/// setters and other DOM algorithms that reject doctype targets per spec.
fn node_is_doctype<'s>(scope: &mut v8::PinScope<'s, '_>, node: v8::Local<'s, v8::Object>) -> bool {
    object_number_property(scope, node, "nodeType")
        .map(|t| t as u32 == 10)
        .unwrap_or(false)
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Range boundary method")]
struct RangeRelativeBoundaryArgs<'s> {
    #[webidl(with = range_relative_boundary_node_arg)]
    node: v8::Local<'s, v8::Object>,
}

fn range_relative_boundary_node_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    webidl_node_arg(scope, args, index, "Range boundary method requires a Node")
}

fn range_set_boundary_relative<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    is_start: bool,
    after: bool,
) {
    let Some(parsed) = webidl::parse_args::<RangeRelativeBoundaryArgs<'s>>(scope, &args) else {
        return;
    };
    let node = parsed.node;
    // Spec setStartBefore/setStartAfter/setEndBefore/setEndAfter(node):
    // If node has no parent, throw InvalidNodeTypeError (the node is not a
    // child of any container, so before/after has no boundary point). WPT
    // tests this explicitly with detached / document-root nodes.
    let Some(parent) = object_property_as_object(scope, node, "parentNode") else {
        throw_named_dom_exception(
            scope,
            "InvalidNodeTypeError",
            "Range boundary method requires a node with a parent.",
        );
        return;
    };
    let Some(index) = child_index(scope, parent, node) else {
        rv.set_undefined();
        return;
    };
    let offset = index + u32::from(after);
    let side = if is_start {
        RangeBoundarySide::Start
    } else {
        RangeBoundarySide::End
    };
    range_set_boundary_and_sync_selection(scope, args.this(), side, parent, offset);
    rv.set_undefined();
}

// Direct and relative setters use the same Range adjustment and Selection
// notification, after their argument and boundary validation has succeeded.
fn range_set_boundary_and_sync_selection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
    container: v8::Local<'s, v8::Object>,
    offset: u32,
) {
    let selection_update = selection_range_update_state(scope, range);
    set_range_boundary(scope, range, side, container, offset);
    let other_side = match side {
        RangeBoundarySide::Start => RangeBoundarySide::End,
        RangeBoundarySide::End => RangeBoundarySide::Start,
    };
    let Some(other_container) = range_boundary_container_object(scope, range, other_side) else {
        return;
    };
    let other_offset = range_boundary_offset(scope, range, other_side) as u32;
    let order = match side {
        RangeBoundarySide::Start => {
            point_order(scope, container, offset, other_container, other_offset)
        }
        RangeBoundarySide::End => {
            point_order(scope, other_container, other_offset, container, offset)
        }
    };
    if order.is_none_or(|order| order == std::cmp::Ordering::Greater) {
        set_range_boundary(scope, range, other_side, container, offset);
    }
    match side {
        RangeBoundarySide::Start => {
            range_sync_selection_after_set_start(scope, selection_update, range, container, offset)
        }
        RangeBoundarySide::End => {
            range_sync_selection_after_set_end(scope, selection_update, range, container, offset)
        }
    }
}

fn range_sync_selection_after_set_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection_update: Option<SelectionRangeUpdateState<'s>>,
    range: v8::Local<'s, v8::Object>,
    new_start_node: v8::Local<'s, v8::Object>,
    new_start_offset: u32,
) {
    let Some(selection_update) = selection_update else {
        return;
    };
    let old_end_node = selection_update.old_composed_end_node;
    let old_end_offset = selection_update.old_composed_end_offset;
    let collapse_composed = selection_composed_boundary_order(
        scope,
        new_start_node,
        new_start_offset,
        old_end_node,
        old_end_offset,
    )
    .is_none_or(|order| order == std::cmp::Ordering::Greater);
    let (composed_start_node, composed_start_offset, composed_end_node, composed_end_offset) =
        if collapse_composed {
            (
                new_start_node,
                new_start_offset,
                new_start_node,
                new_start_offset,
            )
        } else {
            (
                new_start_node,
                new_start_offset,
                old_end_node,
                old_end_offset,
            )
        };
    selection_sync_associated_range(
        scope,
        selection_update,
        range,
        composed_start_node,
        composed_start_offset,
        composed_end_node,
        composed_end_offset,
    );
}

fn range_sync_selection_after_set_end<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection_update: Option<SelectionRangeUpdateState<'s>>,
    range: v8::Local<'s, v8::Object>,
    new_end_node: v8::Local<'s, v8::Object>,
    new_end_offset: u32,
) {
    let Some(selection_update) = selection_update else {
        return;
    };
    let old_start_node = selection_update.old_composed_start_node;
    let old_start_offset = selection_update.old_composed_start_offset;
    let collapse_composed = selection_composed_boundary_order(
        scope,
        old_start_node,
        old_start_offset,
        new_end_node,
        new_end_offset,
    )
    .is_none_or(|order| order == std::cmp::Ordering::Greater);
    let (composed_start_node, composed_start_offset, composed_end_node, composed_end_offset) =
        if collapse_composed {
            (new_end_node, new_end_offset, new_end_node, new_end_offset)
        } else {
            (
                old_start_node,
                old_start_offset,
                new_end_node,
                new_end_offset,
            )
        };
    selection_sync_associated_range(
        scope,
        selection_update,
        range,
        composed_start_node,
        composed_start_offset,
        composed_end_node,
        composed_end_offset,
    );
}

fn range_sync_selection_to_current_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection_update: Option<SelectionRangeUpdateState<'s>>,
    range: v8::Local<'s, v8::Object>,
) {
    let Some(selection_update) = selection_update else {
        return;
    };
    let Some(start_node) = range_boundary_container_object(scope, range, RangeBoundarySide::Start)
    else {
        selection_clear(scope, selection_update.selection);
        return;
    };
    let Some(end_node) = range_boundary_container_object(scope, range, RangeBoundarySide::End)
    else {
        selection_clear(scope, selection_update.selection);
        return;
    };
    let start_offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32;
    let end_offset = range_boundary_offset(scope, range, RangeBoundarySide::End) as u32;
    selection_sync_associated_range(
        scope,
        selection_update,
        range,
        start_node,
        start_offset,
        end_node,
        end_offset,
    );
}
