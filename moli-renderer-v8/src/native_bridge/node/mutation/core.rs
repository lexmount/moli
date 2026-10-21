use super::fragment::validate_document_sequence;
use super::*;
use crate::custom_elements;
use crate::native_bridge::document::is_attr_node_value;

#[derive(Clone, Copy)]
enum TreeInsertionNodeArgument {
    // Attr implements Node, but attributes are stored outside the tree arena.
    // Preserve it through argument conversion until insertion type validation.
    Attribute,
    TreeNode(DomHandle),
}

impl TreeInsertionNodeArgument {
    fn native_handle(self) -> Option<DomHandle> {
        match self {
            Self::Attribute => None,
            Self::TreeNode(handle) => Some(handle),
        }
    }

    fn require_tree_handle(self, scope: &mut v8::PinScope<'_, '_>) -> Option<DomHandle> {
        let Some(handle) = self.native_handle() else {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return None;
        };
        Some(handle)
    }
}

fn required_tree_insertion_node_arg(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: Option<DomHandle>,
    value: v8::Local<'_, v8::Value>,
    invalid_type_message: &'static str,
) -> Option<TreeInsertionNodeArgument> {
    if is_attr_node_value(scope, value) {
        return Some(TreeInsertionNodeArgument::Attribute);
    }
    let Some(handle) =
        node_or_foreign_arg_handle_allow_detached(scope, runtime_ptr, document_handle, value)
    else {
        throw_type_error(scope, invalid_type_message);
        return None;
    };
    Some(TreeInsertionNodeArgument::TreeNode(handle))
}

pub(in crate::native_bridge) fn node_append_child_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = NativeNodeReference::receiver(scope, args.this(), "appendChild") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<AppendChildArgs>(scope, &args) else {
        return;
    };
    let Some(parent) = receiver.tree_node() else {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    };
    let detached = node_runtime_and_handle_from_object(scope, receiver.object()).is_err();
    let (runtime_ptr, parent) = (parent.runtime_ptr, parent.handle);
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(child) = required_tree_insertion_node_arg(
        scope,
        runtime_ptr,
        document_handle,
        parsed.node.into(),
        "Failed to execute 'appendChild' on 'Node': parameter 1 is not of type 'Node'.",
    ) else {
        return;
    };
    let Some(child) = child.require_tree_handle(scope) else {
        return;
    };
    if !validate_pre_insert_handles(scope, unsafe { &*runtime_ptr }, parent, child, None, &[]) {
        return;
    }
    let inserted =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let runtime = unsafe { &mut *runtime_ptr };
            if detached {
                runtime.insert_detached_native_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    child,
                    None,
                )
            } else {
                runtime.append_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    child,
                )
            }
        });
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    rv.set(parsed.node.into());
}

pub(in crate::native_bridge) fn node_insert_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = NativeNodeReference::receiver(scope, args.this(), "insertBefore") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<InsertBeforeArgs>(scope, &args) else {
        return;
    };
    let Some(parent) = receiver.tree_node() else {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    };
    let detached = node_runtime_and_handle_from_object(scope, receiver.object()).is_err();
    let (runtime_ptr, parent) = (parent.runtime_ptr, parent.handle);
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(child) = required_tree_insertion_node_arg(
        scope,
        runtime_ptr,
        document_handle,
        parsed.node.into(),
        "Failed to execute 'insertBefore' on 'Node': parameter 1 is not of type 'Node'.",
    ) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !validate_pre_insert_parent_and_ancestor(scope, runtime, parent, child.native_handle()) {
        return;
    }
    let reference_child = if let Some(reference_child) = parsed.child {
        match existing_node_arg(scope, runtime_ptr, reference_child.into()) {
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
    } else {
        None
    };
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
    let Some(child) = child.require_tree_handle(scope) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !validate_pre_insert_node_type_and_document(
        scope,
        runtime,
        parent,
        child,
        reference_child,
        &[],
    ) {
        return;
    }
    let inserted =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let runtime = unsafe { &mut *runtime_ptr };
            if detached {
                runtime.insert_detached_native_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    child,
                    reference_child,
                )
            } else {
                runtime.insert_before_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    child,
                    reference_child,
                )
            }
        });
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    rv.set(parsed.node.into());
}

pub(crate) fn validate_pre_insert_handles(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    parent: DomHandle,
    child: DomHandle,
    reference_child: Option<DomHandle>,
    skipped: &[DomHandle],
) -> bool {
    if !validate_pre_insert_parent_and_ancestor(scope, runtime, parent, Some(child)) {
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
    child: Option<DomHandle>,
) -> bool {
    if !node_can_contain_children(runtime, parent)
        || child
            .is_some_and(|child| node_is_host_including_inclusive_ancestor(runtime, child, parent))
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
    let parent_is_document = node_is_document(runtime, parent);
    if !node_type_is_insertable(child_node_type)
        || (node_type_is_text_like(child_node_type) && parent_is_document)
        || (child_node_type == NodeType::DocumentType && !parent_is_document)
    {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    if !parent_is_document {
        return true;
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
        unsafe { &mut *runtime_ptr }
            .move_before_preserving_state_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                child,
                reference_child,
            )
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
    let Some(receiver) = NativeNodeReference::receiver(scope, args.this(), "removeChild") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<RequiredNodeArgs>(scope, &args) else {
        return;
    };
    let Some(parent) = receiver.tree_node() else {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node to be removed is not a child of this node.",
        );
        return;
    };
    let (runtime_ptr, parent) = (parent.runtime_ptr, parent.handle);
    let child = match existing_node_arg(scope, runtime_ptr, parsed.node.into()) {
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
            unsafe { &mut *runtime_ptr }.remove_child_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                child,
            )
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
    rv.set(parsed.node.into());
}

pub(in crate::native_bridge) fn node_replace_child_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = NativeNodeReference::receiver(scope, args.this(), "replaceChild") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<ReplaceChildArgs>(scope, &args) else {
        return;
    };
    let Some(parent) = receiver.tree_node() else {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    };
    let detached = node_runtime_and_handle_from_object(scope, receiver.object()).is_err();
    if replace_node_child(
        scope,
        parent.runtime_ptr,
        parent.handle,
        parsed.node,
        parsed.child,
        detached,
    )
    .is_some()
    {
        rv.set(parsed.child.into());
    }
}

fn replace_node_child<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    parent: DomHandle,
    new_child_value: v8::Local<'s, v8::Object>,
    old_child_value: v8::Local<'s, v8::Object>,
    detached: bool,
) -> Option<DomHandle> {
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let new_child = required_tree_insertion_node_arg(
        scope,
        runtime_ptr,
        document_handle,
        new_child_value.into(),
        "Failed to execute 'replaceChild' on 'Node': parameter 1 is not of type 'Node'.",
    )?;
    let runtime = unsafe { &*runtime_ptr };
    if !validate_pre_insert_parent_and_ancestor(scope, runtime, parent, new_child.native_handle()) {
        return None;
    }
    let old_child = match existing_node_arg(scope, runtime_ptr, old_child_value.into()) {
        ExistingNodeArgument::Handle(old_child) => old_child,
        ExistingNodeArgument::ForeignNode => {
            throw_dom_exception(
                scope,
                "NotFoundError",
                8,
                "The node to be replaced is not a child of this node.",
            );
            return None;
        }
        ExistingNodeArgument::Invalid => {
            throw_type_error(
                scope,
                "Failed to execute 'replaceChild' on 'Node': parameter 2 is not of type 'Node'.",
            );
            return None;
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
        return None;
    }
    let new_child = new_child.require_tree_handle(scope)?;
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
        return None;
    }
    let replaced =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let runtime = unsafe { &mut *runtime_ptr };
            if detached {
                runtime.replace_detached_native_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    new_child,
                    old_child,
                )
            } else {
                runtime.replace_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    new_child,
                    old_child,
                )
            }
        });
    if !replaced {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return None;
    }
    Some(old_child)
}
