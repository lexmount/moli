use super::fragment::validate_document_sequence;
use super::*;
use crate::custom_elements;
use crate::native_bridge::document::{
    detached_native_handle_for_runtime, is_attr_node_value,
    paired_detached_native_object_for_handle,
};

fn required_tree_insertion_node_arg_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: Option<DomHandle>,
    value: v8::Local<'_, v8::Value>,
    invalid_type_message: &'static str,
) -> Option<DomHandle> {
    if is_attr_node_value(scope, value) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return None;
    }
    let Some(handle) =
        node_or_foreign_arg_handle_allow_detached(scope, runtime_ptr, document_handle, value)
    else {
        throw_type_error(scope, invalid_type_message);
        return None;
    };
    Some(handle)
}

fn set_original_node_arg_or_wrapped_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'s, v8::Value>,
    handle: DomHandle,
) {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && detached_native_handle_for_runtime(scope, runtime_ptr, object)
            .is_some_and(|object_handle| object_handle == handle)
    {
        rv.set(object.into());
        return;
    }
    set_wrapped_node_or_null(scope, rv, runtime_ptr, Some(handle));
}

fn inserted_handles_for_post_insert_events(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Vec<DomHandle> {
    if runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document_fragment)
    {
        runtime.dom_host().child_handles(handle).collect()
    } else {
        vec![handle]
    }
}

fn dispatch_detached_post_insert_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handles: &[DomHandle],
) {
    for handle in handles {
        if !unsafe { &*runtime_ptr }
            .dom_host()
            .is_html_element_named(*handle, "iframe")
        {
            continue;
        }
        let Some(target) = paired_detached_native_object_for_handle(scope, runtime_ptr, *handle)
        else {
            continue;
        };
        let _ = crate::detached_event_target::dispatch_detached_simple_event(
            scope, target, "load", false, false, false,
        );
    }
}

pub(in crate::native_bridge) fn node_append_child_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(child) = required_tree_insertion_node_arg_handle(
        scope,
        runtime_ptr,
        document_handle,
        args.get(0),
        "Failed to execute 'appendChild' on 'Node': parameter 1 is not of type 'Node'.",
    ) else {
        return;
    };
    let child_value = args.get(0);
    let post_insert_event_handles =
        inserted_handles_for_post_insert_events(unsafe { &*runtime_ptr }, child);
    let inserted =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if !unsafe { &mut *runtime_ptr }.append_child_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                child,
            ) {
                return false;
            }
            true
        });
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    dispatch_detached_post_insert_events(scope, runtime_ptr, &post_insert_event_handles);
    set_original_node_arg_or_wrapped_handle(scope, &mut rv, runtime_ptr, child_value, child);
}

pub(in crate::native_bridge) fn node_insert_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(child) = required_tree_insertion_node_arg_handle(
        scope,
        runtime_ptr,
        document_handle,
        args.get(0),
        "Failed to execute 'insertBefore' on 'Node': parameter 1 is not of type 'Node'.",
    ) else {
        return;
    };
    if args.length() < 2 {
        throw_type_error(
            scope,
            "Failed to execute 'insertBefore' on 'Node': 2 arguments required.",
        );
        return;
    }
    let reference_child = if args.get(1).is_null_or_undefined() {
        None
    } else {
        match existing_node_arg(scope, runtime_ptr, args.get(1)) {
            ExistingNodeArgument::Handle(reference_child) => Some(reference_child),
            ExistingNodeArgument::ForeignNode => {
                throw_dom_exception(
                    scope,
                    "NotFoundError",
                    8,
                    "The node before which the new node is to be inserted is not a child of this node.",
                );
                return;
            }
            ExistingNodeArgument::Invalid => {
                throw_type_error(
                    scope,
                    "Failed to execute 'insertBefore' on 'Node': parameter 2 is not of type 'Node'.",
                );
                return;
            }
        }
    };
    let runtime = unsafe { &*runtime_ptr };
    if !validate_pre_insert_parent_and_ancestor(scope, runtime, parent, child) {
        return;
    }
    if let Some(reference_child) = reference_child {
        let is_child_of_parent = unsafe { &*runtime_ptr }
            .dom_host()
            .node(reference_child)
            .and_then(Node::parent_node)
            .is_some_and(|current_parent| current_parent == parent);
        if !is_child_of_parent {
            throw_dom_exception(
                scope,
                "NotFoundError",
                8,
                "The node before which the new node is to be inserted is not a child of this node.",
            );
            return;
        }
    }
    let runtime = unsafe { &*runtime_ptr };
    if !validate_pre_insert_node_type_and_document(
        scope,
        runtime,
        parent,
        child,
        reference_child,
        &[child],
    ) {
        return;
    }
    let child_value = args.get(0);
    let post_insert_event_handles =
        inserted_handles_for_post_insert_events(unsafe { &*runtime_ptr }, child);
    let inserted =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if !unsafe { &mut *runtime_ptr }.insert_before_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                child,
                reference_child,
            ) {
                return false;
            }
            true
        });
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    dispatch_detached_post_insert_events(scope, runtime_ptr, &post_insert_event_handles);
    set_original_node_arg_or_wrapped_handle(scope, &mut rv, runtime_ptr, child_value, child);
}

pub(crate) fn validate_pre_insert_handles(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    parent: DomHandle,
    child: DomHandle,
    reference_child: Option<DomHandle>,
    skipped: &[DomHandle],
) -> bool {
    if !validate_pre_insert_parent_and_ancestor(scope, runtime, parent, child) {
        return false;
    }
    if let Some(reference_child) = reference_child {
        let is_child_of_parent = runtime
            .dom_host()
            .node(reference_child)
            .and_then(Node::parent_node)
            .is_some_and(|current_parent| current_parent == parent);
        if !is_child_of_parent {
            throw_dom_exception(
                scope,
                "NotFoundError",
                8,
                "The node before which the new node is to be inserted is not a child of this node.",
            );
            return false;
        }
    }
    validate_pre_insert_node_type_and_document(
        scope,
        runtime,
        parent,
        child,
        reference_child,
        skipped,
    )
}

fn validate_pre_insert_parent_and_ancestor(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    parent: DomHandle,
    child: DomHandle,
) -> bool {
    if !node_can_contain_children(runtime, parent)
        || node_is_host_including_inclusive_ancestor(runtime, child, parent)
    {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    true
}

fn validate_pre_insert_node_type_and_document(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    parent: DomHandle,
    child: DomHandle,
    reference_child: Option<DomHandle>,
    skipped: &[DomHandle],
) -> bool {
    let Some(child_node_type) = runtime.dom_host().node(child).map(Node::node_type) else {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    };
    if !node_type_is_insertable(child_node_type)
        || (node_type_is_text_like(child_node_type) && node_is_document(runtime, parent))
        || (child_node_type == NodeType::DocumentType && !node_is_document(runtime, parent))
    {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }

    let inserted = pre_insert_validation_handles(runtime, child);
    if !validate_document_sequence(runtime, parent, reference_child, &inserted, skipped) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    true
}

pub(super) fn node_can_contain_children(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().node(handle).is_some_and(|node| {
        matches!(
            node.node_type(),
            NodeType::Document | NodeType::DocumentFragment | NodeType::Element
        )
    })
}

pub(super) fn node_type_is_insertable(node_type: NodeType) -> bool {
    matches!(
        node_type,
        NodeType::DocumentFragment
            | NodeType::DocumentType
            | NodeType::Element
            | NodeType::Text
            | NodeType::CDataSection
            | NodeType::ProcessingInstruction
            | NodeType::Comment
    )
}

fn node_type_is_text_like(node_type: NodeType) -> bool {
    matches!(node_type, NodeType::Text | NodeType::CDataSection)
}

pub(super) fn pre_insert_validation_handles(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Vec<DomHandle> {
    if runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document_fragment)
    {
        runtime.dom_host().child_handles(handle).collect()
    } else {
        vec![handle]
    }
}

pub(in crate::native_bridge) fn node_move_before_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "ParentNode", "moveBefore");
        return;
    };
    if !require_parent_node_receiver(scope, unsafe { &*runtime_ptr }, parent, "moveBefore", true) {
        return;
    }
    if args.length() < 2 {
        throw_type_error(
            scope,
            "Failed to execute 'moveBefore' on 'Node': 2 arguments required.",
        );
        return;
    }
    if is_attr_node_value(scope, args.get(0)) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    let Some(child) = node_or_existing_detached_arg_handle(scope, runtime_ptr, args.get(0)) else {
        throw_type_error(
            scope,
            "Failed to execute 'moveBefore' on 'Node': parameter 1 is not of type 'Node'.",
        );
        return;
    };
    let reference_child = if args.get(1).is_null_or_undefined() {
        None
    } else if is_attr_node_value(scope, args.get(1)) {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node before which the node is to be moved is not a child of this node.",
        );
        return;
    } else {
        let Some(reference_child) =
            node_or_existing_detached_arg_handle(scope, runtime_ptr, args.get(1))
        else {
            throw_type_error(
                scope,
                "Failed to execute 'moveBefore' on 'Node': parameter 2 is not of type 'Node'.",
            );
            return;
        };
        Some(reference_child)
    };
    node_move_before_after_argument_validation(scope, runtime_ptr, parent, child, reference_child);
}

fn node_move_before_after_argument_validation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    parent: DomHandle,
    child: DomHandle,
    reference_child: Option<DomHandle>,
) {
    let runtime = unsafe { &*runtime_ptr };
    if !node_move_before_is_valid_parent(runtime, parent)
        || !node_move_before_is_valid_child(runtime, child)
        || node_is_host_including_inclusive_ancestor(runtime, child, parent)
        || node_move_before_shadow_including_root(runtime, parent)
            != node_move_before_shadow_including_root(runtime, child)
    {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    if let Some(reference_child) = reference_child
        && runtime
            .dom_host()
            .node(reference_child)
            .and_then(Node::parent_node)
            != Some(parent)
    {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node before which the node is to be moved is not a child of this node.",
        );
        return;
    }
    if reference_child == Some(child) {
        return;
    }
    if !validate_document_sequence(runtime, parent, reference_child, &[child], &[child]) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }

    let moved = custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        if !unsafe { &mut *runtime_ptr }
            .move_before_preserving_state_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                child,
                reference_child,
            )
        {
            return false;
        }
        true
    });
    if !moved {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
    }
}

fn node_move_before_is_valid_parent(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().node(handle).is_some_and(|node| {
        matches!(
            node.node_type(),
            NodeType::Document | NodeType::DocumentFragment | NodeType::Element
        )
    })
}

fn node_move_before_is_valid_child(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().node(handle).is_some_and(|node| {
        matches!(
            node.node_type(),
            NodeType::Element
                | NodeType::Text
                | NodeType::CDataSection
                | NodeType::ProcessingInstruction
                | NodeType::Comment
        )
    })
}

pub(super) fn node_is_host_including_inclusive_ancestor(
    runtime: &JsContextHost,
    ancestor: DomHandle,
    node: DomHandle,
) -> bool {
    runtime
        .dom_host()
        .is_host_including_inclusive_ancestor(ancestor, node)
}

fn node_move_before_shadow_including_root(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let mut root = runtime.dom_host().root_node_handle(handle)?;
    while runtime.dom_host().is_shadow_root(root) {
        let host = runtime.dom_host().shadow_root_host(root)?;
        root = runtime.dom_host().root_node_handle(host)?;
    }
    Some(root)
}

pub(in crate::native_bridge) fn node_remove_child_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    let child = match existing_node_arg(scope, runtime_ptr, args.get(0)) {
        ExistingNodeArgument::Handle(child) => child,
        ExistingNodeArgument::ForeignNode => {
            throw_dom_exception(
                scope,
                "NotFoundError",
                8,
                "The node to be removed is not a child of this node.",
            );
            return;
        }
        ExistingNodeArgument::Invalid => {
            throw_type_error(
                scope,
                "Failed to execute 'removeChild' on 'Node': parameter 1 is not of type 'Node'.",
            );
            return;
        }
    };
    let is_child_of_parent = unsafe { &*runtime_ptr }
        .dom_host()
        .node(child)
        .and_then(Node::parent_node)
        .is_some_and(|current_parent| current_parent == parent);
    if !is_child_of_parent {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node to be removed is not a child of this node.",
        );
        return;
    }
    let removed =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if !unsafe { &mut *runtime_ptr }.remove_child_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                child,
            ) {
                return false;
            }
            true
        });
    if !removed {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node to be removed is not a child of this node.",
        );
        return;
    }
    set_original_node_arg_or_wrapped_handle(scope, &mut rv, runtime_ptr, args.get(0), child);
}

pub(in crate::native_bridge) fn node_replace_child_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let new_child_value = args.get(0);
    let Some(new_child) = required_tree_insertion_node_arg_handle(
        scope,
        runtime_ptr,
        document_handle,
        new_child_value,
        "Failed to execute 'replaceChild' on 'Node': parameter 1 is not of type 'Node'.",
    ) else {
        return;
    };
    if args.length() < 2 {
        throw_type_error(
            scope,
            "Failed to execute 'replaceChild' on 'Node': 2 arguments required.",
        );
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    if !validate_pre_insert_parent_and_ancestor(scope, runtime, parent, new_child) {
        return;
    }
    let old_child = match existing_node_arg(scope, runtime_ptr, args.get(1)) {
        ExistingNodeArgument::Handle(old_child) => old_child,
        ExistingNodeArgument::ForeignNode => {
            throw_dom_exception(
                scope,
                "NotFoundError",
                8,
                "The node to be replaced is not a child of this node.",
            );
            return;
        }
        ExistingNodeArgument::Invalid => {
            throw_type_error(
                scope,
                "Failed to execute 'replaceChild' on 'Node': parameter 2 is not of type 'Node'.",
            );
            return;
        }
    };
    let is_child_of_parent = unsafe { &*runtime_ptr }
        .dom_host()
        .node(old_child)
        .and_then(Node::parent_node)
        .is_some_and(|current_parent| current_parent == parent);
    if !is_child_of_parent {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node to be replaced is not a child of this node.",
        );
        return;
    }
    let skipped = if new_child == old_child {
        vec![old_child]
    } else {
        vec![old_child, new_child]
    };
    let runtime = unsafe { &*runtime_ptr };
    if !validate_pre_insert_node_type_and_document(
        scope,
        runtime,
        parent,
        new_child,
        Some(old_child),
        &skipped,
    ) {
        return;
    }
    let post_insert_event_handles =
        inserted_handles_for_post_insert_events(unsafe { &*runtime_ptr }, new_child);
    let replaced =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if !unsafe { &mut *runtime_ptr }.replace_child_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                new_child,
                old_child,
            ) {
                return false;
            }
            true
        });
    if !replaced {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    dispatch_detached_post_insert_events(scope, runtime_ptr, &post_insert_event_handles);
    set_original_node_arg_or_wrapped_handle(scope, &mut rv, runtime_ptr, args.get(1), old_child);
}
