use super::*;
use crate::native_bridge::callback_value_dom_handle;
use crate::native_bridge::element::{contenteditable_editing_host, focus_element};
use crate::native_bridge::throw_dom_exception;
use crate::webidl;

/// Throw a DOMException with the IndexSizeError name and code (1).
fn throw_index_size_error(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    throw_dom_exception(scope, "IndexSizeError", 1, message);
}

/// Throw a DOMException with the InvalidNodeTypeError name and code (24).
fn throw_invalid_node_type_error(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    throw_dom_exception(scope, "InvalidNodeTypeError", 24, message);
}

/// Throw a DOMException with the InvalidStateError name and code (11).
fn throw_invalid_state_error(scope: &mut v8::PinScope<'_, '_>, message: &'static str) {
    throw_dom_exception(scope, "InvalidStateError", 11, message);
}

/// True iff `a` and `b` share the same tree root. Walks parentNode chains and
/// crosses ShadowRoot.host links before comparing the topmost ancestors. Used
/// by Selection.extend to determine whether a node belongs to this selection's
/// shadow-including tree.
fn nodes_share_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    a: v8::Local<'s, v8::Object>,
    b: v8::Local<'s, v8::Object>,
) -> bool {
    let a_root = selection_shadow_including_tree_root(scope, a);
    let b_root = selection_shadow_including_tree_root(scope, b);
    a_root.strict_equals(b_root.into())
}

fn nodes_share_dom_tree_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    a: v8::Local<'s, v8::Object>,
    b: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(a) = callback_value_dom_handle(scope, a.into()) else {
        return false;
    };
    let Some(b) = callback_value_dom_handle(scope, b.into()) else {
        return false;
    };
    let dom = unsafe { &*host_ptr }.dom_host();
    dom.root_node_handle(a)
        .is_some_and(|root| dom.root_node_handle(b) == Some(root))
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Selection.collapse")]
struct SelectionCollapseArgs<'s> {
    #[webidl(with = selection_collapse_node_arg)]
    node: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = 0)]
    offset: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Selection.extend")]
struct SelectionExtendArgs<'s> {
    #[webidl(with = selection_extend_focus_node_arg)]
    focus_node: v8::Local<'s, v8::Object>,
    #[webidl(default = 0)]
    focus_offset: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Selection.selectAllChildren")]
struct SelectionSelectAllChildrenArgs<'s> {
    #[webidl(with = selection_select_all_children_node_arg)]
    node: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Selection.setBaseAndExtent")]
struct SelectionSetBaseAndExtentArgs<'s> {
    #[webidl(with = selection_anchor_node_arg)]
    anchor_node: v8::Local<'s, v8::Object>,
    #[webidl(required)]
    anchor_offset: u32,
    #[webidl(with = selection_focus_node_arg)]
    focus_node: v8::Local<'s, v8::Object>,
    #[webidl(required)]
    focus_offset: u32,
}

fn selection_required_node_arg<'s>(
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    message: &'static str,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    v8::Local::<v8::Object>::try_from(args.get(index))
        .map_err(|_| webidl::WebIdlError::custom_message(message))
}

fn selection_collapse_node_arg<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    let value = args.get(index);
    if value.is_null() {
        return Ok(None);
    }
    selection_required_node_arg(args, index, "Selection.collapse requires a Node or null").map(Some)
}

fn selection_extend_focus_node_arg<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    selection_required_node_arg(args, index, "Selection.extend requires a Node")
}

fn selection_select_all_children_node_arg<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    selection_required_node_arg(args, index, "Selection.selectAllChildren requires a Node")
}

fn selection_anchor_node_arg<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    selection_required_node_arg(
        args,
        index,
        "Selection.setBaseAndExtent requires anchorNode",
    )
}

fn selection_focus_node_arg<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    selection_required_node_arg(args, index, "Selection.setBaseAndExtent requires focusNode")
}

pub(in crate::context_bootstrap) fn selection_collapse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SelectionCollapseArgs>(scope, &args) else {
        return;
    };
    let Some(node) = parsed.node else {
        if selection_has_range(scope, args.this()) {
            selection_clear(scope, args.this());
            selection_dispatch_change(scope);
        }
        return;
    };
    // DOM spec Selection.collapse(node, offset) — step order matches WPT
    // selection/collapse.js:
    //   1. If node is a DocumentType, throw InvalidNodeTypeError.
    //   2. If offset < 0 or > node.length, throw IndexSizeError.
    //   3. If node's root is not this Selection's document, no-op silently.
    //   4. Otherwise update the selection.
    let node_type = object_number_property(scope, node, "nodeType").unwrap_or(0.0) as u32;
    if node_type == 10 {
        throw_invalid_node_type_error(
            scope,
            "Selection.collapse target must not be a DocumentType.",
        );
        return;
    }
    if !range_validate_boundary_point(scope, node, parsed.offset) {
        throw_index_size_error(scope, "Index or offset is out of range.");
        return;
    }
    // No-op silently when node isn't part of the document tree (detached
    // element / foreign-doc element). WPT explicitly asserts that
    // anchorNode/anchorOffset stay at their pre-call values, so we must
    // NOT mutate state here.
    if !node_is_in_selection_document(scope, args.this(), node) {
        return;
    }
    if selection_set_collapsed(scope, args.this(), node, parsed.offset) {
        selection_focus_editing_host_for_boundary(scope, node);
        selection_dispatch_change(scope);
    }
}

/// True iff `node` is part of the main browsing-context document tree —
/// i.e. walking parentNode upward reaches the global `document` Object. Used
/// by Selection.collapse / setPosition / extend silent-no-op checks.
fn node_is_in_selection_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    selection: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    let document = selection_owner_document(scope, selection).or_else(|| {
        scope
            .get_current_context()
            .global(scope)
            .get(scope, crate::util::v8str(scope, "document").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    });
    let Some(document) = document else {
        return true; // best-effort: allow if document isn't reachable
    };
    let root = selection_shadow_including_tree_root(scope, node);
    root.strict_equals(document.into())
}

fn selection_focus_editing_host_for_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(handle) = callback_value_dom_handle(scope, node.into()) else {
        return;
    };
    let Some(editing_host) = contenteditable_editing_host(unsafe { &*runtime_ptr }, handle) else {
        return;
    };
    focus_element(scope, runtime_ptr, editing_host);
}

fn selection_shadow_including_tree_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let mut current = node;
    loop {
        if let Some(parent) = object_property_as_object(scope, current, "parentNode")
            && !parent.strict_equals(current.into())
        {
            current = parent;
            continue;
        }
        if object_number_property(scope, current, "nodeType").unwrap_or(0.0) as u32 == 11
            && let Some(host) = object_property_as_object(scope, current, "host")
        {
            current = host;
            continue;
        }
        return current;
    }
}

pub(in crate::context_bootstrap) fn selection_set_position_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    selection_collapse_callback(scope, args, rv);
}

pub(in crate::context_bootstrap) fn selection_collapse_to_start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(range) = selection_range(scope, args.this()) else {
        throw_invalid_state_error(scope, "Selection has no range.");
        return;
    };
    let Some(start_node) = range_boundary_container_object(scope, range, RangeBoundarySide::Start)
    else {
        return;
    };
    let start_offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32;
    if selection_set_collapsed(scope, args.this(), start_node, start_offset) {
        selection_dispatch_change(scope);
    }
}

pub(in crate::context_bootstrap) fn selection_collapse_to_end_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(range) = selection_range(scope, args.this()) else {
        throw_invalid_state_error(scope, "Selection has no range.");
        return;
    };
    let Some(end_node) = range_boundary_container_object(scope, range, RangeBoundarySide::End)
    else {
        return;
    };
    let end_offset = range_boundary_offset(scope, range, RangeBoundarySide::End) as u32;
    if selection_set_collapsed(scope, args.this(), end_node, end_offset) {
        selection_dispatch_change(scope);
    }
}

pub(in crate::context_bootstrap) fn selection_extend_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(anchor_node) = selection_anchor_node(scope, args.this()) else {
        throw_invalid_state_error(scope, "Selection has no range.");
        return;
    };
    let Some(parsed) = webidl::parse_args::<SelectionExtendArgs>(scope, &args) else {
        return;
    };

    // DOM spec Selection.extend(node, offset):
    // Step 1: If node's root is not the same as this's range's root, return.
    // (Silently — no exception, no state change. WPT verifies that focusNode
    //  is unchanged when extending to a node in a detached tree.)
    if !nodes_share_root(scope, anchor_node, parsed.focus_node) {
        return;
    }
    // Step 2: If node is a doctype, throw InvalidNodeTypeError.
    let node_type =
        object_number_property(scope, parsed.focus_node, "nodeType").unwrap_or(0.0) as u32;
    if node_type == 10 {
        throw_invalid_node_type_error(scope, "Selection.extend focus must not be a DocumentType.");
        return;
    }
    // Step 3: validate offset is within node length.
    if !range_validate_boundary_point(scope, parsed.focus_node, parsed.focus_offset) {
        throw_index_size_error(scope, "Index or offset is out of range.");
        return;
    }

    // An ordinary Range cannot span distinct DOM roots. Extending into a
    // different shadow tree replaces it with a caret at the new focus.
    let (anchor_node, anchor_offset) =
        if nodes_share_dom_tree_root(scope, anchor_node, parsed.focus_node) {
            (anchor_node, selection_anchor_offset(scope, args.this()))
        } else {
            (parsed.focus_node, parsed.focus_offset)
        };
    let direction = match boundary_order(
        scope,
        anchor_node,
        anchor_offset,
        parsed.focus_node,
        parsed.focus_offset,
    ) {
        std::cmp::Ordering::Less => "forward",
        std::cmp::Ordering::Greater => "backward",
        std::cmp::Ordering::Equal => "none",
    };
    // Step 4 (paraphrased): extend always creates a FRESH range — WPT asserts
    // assert_not_equals(newRange, oldRange) to enforce that reusing the
    // existing live-range object is a spec violation. The anchor side of the
    // range comes from the existing anchor, the focus side from the argument.
    let Some(document) = node_owner_document_or_self(scope, anchor_node) else {
        return;
    };
    let Some(range) = new_range_for_document(scope, document) else {
        return;
    };
    selection_store(
        scope,
        args.this(),
        range,
        anchor_node,
        anchor_offset,
        parsed.focus_node,
        parsed.focus_offset,
        direction,
    );
    selection_dispatch_change(scope);
}

pub(in crate::context_bootstrap) fn selection_select_all_children_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SelectionSelectAllChildrenArgs>(scope, &args) else {
        return;
    };
    // Spec (matches WPT selectAllChildren.html):
    //   1. If node is a doctype, throw InvalidNodeTypeError. (WPT asserts
    //      INVALID_NODE_TYPE_ERR — legacy DOM4 semantics the test suite
    //      carries forward; the modern spec name is HierarchyRequestError
    //      but the test asserts the legacy code.)
    //   2. If node is not contained in this Selection's document, no-op
    //      silently. Detached subtrees AND foreign-document nodes (e.g.
    //      createHTMLDocument().createElement(...) results) both fall here.
    //   3. Otherwise set the selection to span (node, 0) -> (node, count).
    //      "Count" is childNodes.length per WPT — CharacterData nodes have
    //      zero children even though their data has a length.
    let node_type = object_number_property(scope, parsed.node, "nodeType").unwrap_or(0.0) as u32;
    if node_type == 10 {
        throw_invalid_node_type_error(
            scope,
            "Selection.selectAllChildren cannot select children of a DocumentType.",
        );
        return;
    }
    if !node_is_in_selection_document(scope, args.this(), parsed.node) {
        return;
    }
    let Some(document) = node_owner_document_or_self(scope, parsed.node) else {
        return;
    };
    let Some(range) = new_range_for_document(scope, document) else {
        return;
    };
    let end_offset = crate::util::object_property_as_object(scope, parsed.node, "childNodes")
        .and_then(|child_nodes| {
            object_number_property(scope, child_nodes, "length").map(|len| len as u32)
        })
        .unwrap_or(0);
    let direction = if end_offset == 0 { "none" } else { "forward" };
    selection_store(
        scope,
        args.this(),
        range,
        parsed.node,
        0,
        parsed.node,
        end_offset,
        direction,
    );
    selection_dispatch_change(scope);
}

pub(in crate::context_bootstrap) fn selection_set_base_and_extent_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SelectionSetBaseAndExtentArgs>(scope, &args) else {
        return;
    };
    // Spec order matches collapse() + extend() composite:
    //   1. Throw InvalidNodeTypeError if either node is a DocumentType.
    //   2. Throw IndexSizeError if either offset is out of range.
    //   3. Silently no-op if anchor or focus is detached from the document.
    //   4. Otherwise build a fresh range and update.
    let anchor_type =
        object_number_property(scope, parsed.anchor_node, "nodeType").unwrap_or(0.0) as u32;
    let focus_type =
        object_number_property(scope, parsed.focus_node, "nodeType").unwrap_or(0.0) as u32;
    if anchor_type == 10 || focus_type == 10 {
        throw_invalid_node_type_error(
            scope,
            "Selection.setBaseAndExtent endpoints must not be a DocumentType.",
        );
        return;
    }
    if !range_validate_boundary_point(scope, parsed.anchor_node, parsed.anchor_offset)
        || !range_validate_boundary_point(scope, parsed.focus_node, parsed.focus_offset)
    {
        throw_index_size_error(scope, "Index or offset is out of range.");
        return;
    }
    if !node_is_in_selection_document(scope, args.this(), parsed.anchor_node)
        || !node_is_in_selection_document(scope, args.this(), parsed.focus_node)
    {
        return;
    }
    let Some(document) = node_owner_document_or_self(scope, parsed.anchor_node) else {
        return;
    };
    let Some(range) = new_range_for_document(scope, document) else {
        return;
    };
    if nodes_share_dom_tree_root(scope, parsed.anchor_node, parsed.focus_node) {
        let direction = match boundary_order(
            scope,
            parsed.anchor_node,
            parsed.anchor_offset,
            parsed.focus_node,
            parsed.focus_offset,
        ) {
            std::cmp::Ordering::Less => "forward",
            std::cmp::Ordering::Greater => "backward",
            std::cmp::Ordering::Equal => "none",
        };
        selection_store(
            scope,
            args.this(),
            range,
            parsed.anchor_node,
            parsed.anchor_offset,
            parsed.focus_node,
            parsed.focus_offset,
            direction,
        );
    } else {
        let composed_order = selection_composed_boundary_order(
            scope,
            parsed.anchor_node,
            parsed.anchor_offset,
            parsed.focus_node,
            parsed.focus_offset,
        );
        let (composed_start_node, composed_start_offset, composed_end_node, composed_end_offset) =
            match composed_order {
                Some(std::cmp::Ordering::Greater) => (
                    parsed.focus_node,
                    parsed.focus_offset,
                    parsed.anchor_node,
                    parsed.anchor_offset,
                ),
                _ => (
                    parsed.anchor_node,
                    parsed.anchor_offset,
                    parsed.focus_node,
                    parsed.focus_offset,
                ),
            };
        let (collapsed_node, collapsed_offset) = match composed_order {
            Some(std::cmp::Ordering::Greater) => (parsed.anchor_node, parsed.anchor_offset),
            _ => (parsed.focus_node, parsed.focus_offset),
        };
        let direction = match composed_order {
            Some(std::cmp::Ordering::Less) => "forward",
            Some(std::cmp::Ordering::Greater) => "backward",
            _ => "none",
        };
        selection_store_with_composed_boundaries(
            scope,
            args.this(),
            range,
            collapsed_node,
            collapsed_offset,
            collapsed_node,
            collapsed_offset,
            direction,
            collapsed_node,
            collapsed_offset,
            collapsed_node,
            collapsed_offset,
            composed_start_node,
            composed_start_offset,
            composed_end_node,
            composed_end_offset,
        );
    }
    selection_dispatch_change(scope);
}
