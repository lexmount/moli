use super::*;
use crate::native_bridge::node::{
    ChildNodeMutation, ParentNodeMutation, mutate_child_node, mutate_parent_node,
    replace_node_child,
};

fn detached_insert_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
    reference_child: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    match detached_insert_node_appending_to_current_reaction_queue(
        scope,
        parent,
        child,
        reference_child,
    ) {
        Ok(_) => true,
        Err((name, code, message)) => {
            throw_dom_exception(scope, name, code, message);
            false
        }
    }
}

fn detached_pre_insert_hierarchy_error() -> (&'static str, i32, &'static str) {
    (
        "HierarchyRequestError",
        3,
        "The operation would yield an invalid node tree.",
    )
}

fn detached_pre_insert_parent_and_ancestor_are_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
) -> bool {
    detached_move_before_valid_parent(scope, parent)
        && !detached_shadow_including_contains(scope, child, parent)
}

fn detached_pre_insert_node_type_is_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
) -> bool {
    let child_type = detached_node_type(scope, child);
    matches!(child_type, Some(1 | 3 | 4 | 7 | 8 | 10 | 11))
        && !(child_type == Some(3) && detached_node_type(scope, parent) == Some(9))
        && !(child_type == Some(10) && detached_node_type(scope, parent) != Some(9))
}

fn detached_document_direct_element_child_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
) -> bool {
    if detached_node_type(scope, parent) != Some(9) || detached_node_type(scope, child) != Some(1) {
        return false;
    }
    if !detached_owner_document_object(scope, child)
        .is_some_and(|owner| owner.strict_equals(parent.into()))
    {
        return false;
    }
    let child_name = detached_element_local_name(scope, child)
        .or_else(|| object_string_property(scope, child, "localName"));
    for candidate in detached_child_node_objects(scope, parent) {
        if detached_node_type(scope, candidate) != Some(1) {
            continue;
        }
        if candidate.strict_equals(child.into())
            || detached_element_local_name(scope, candidate)
                .or_else(|| object_string_property(scope, candidate, "localName"))
                == child_name
        {
            return true;
        }
    }
    false
}

fn detached_document_replace_would_violate_root_shape<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    new_child: v8::Local<'s, v8::Object>,
    old_child: v8::Local<'s, v8::Object>,
) -> bool {
    if detached_node_type(scope, parent) != Some(9) {
        return false;
    }
    let current_children = detached_child_node_objects(scope, parent);
    let Some(old_child_index) = current_children
        .iter()
        .position(|candidate| candidate.strict_equals(old_child.into()))
    else {
        // Membership is checked separately so that a missing old child keeps
        // producing NotFoundError rather than a document-shape error.
        return false;
    };
    let insertion_nodes = detached_flattened_insertion_nodes(scope, new_child);
    let insertion_index = current_children[..old_child_index]
        .iter()
        .filter(|candidate| {
            !insertion_nodes
                .iter()
                .any(|inserted| candidate.strict_equals((*inserted).into()))
        })
        .count();
    let mut prospective = current_children
        .iter()
        .copied()
        .filter(|candidate| {
            !candidate.strict_equals(old_child.into())
                && !insertion_nodes
                    .iter()
                    .any(|inserted| candidate.strict_equals((*inserted).into()))
        })
        .collect::<Vec<_>>();
    prospective.splice(
        insertion_index..insertion_index,
        insertion_nodes.iter().copied(),
    );
    !detached_validate_document_children(scope, &prospective)
}

pub(in crate::native_bridge) fn bridge_detached_append_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    if mutate_native_detached_parent(scope, &args, target, ParentNodeMutation::Append) {
        return;
    }
    let document = detached_owner_document_object(scope, target).unwrap_or(target);
    let mut values = Vec::new();
    for index in 1..args.length() {
        values.push(args.get(index));
    }
    let Some(nodes) = detached_nodes_from_values(scope, document, &values) else {
        return;
    };
    if !detached_parent_node_pre_insert_validate(scope, target, None, &nodes) {
        return;
    }
    let Some(fragment) = build_fragment_from_nodes(scope, document, &nodes) else {
        return;
    };
    with_detached_tree_reaction_scope(scope, |scope| {
        let _ = detached_insert_or_throw(scope, target, fragment, None);
    });
}

fn mutate_native_detached_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    target: v8::Local<'s, v8::Object>,
    mutation: ParentNodeMutation,
) -> bool {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(parent) = detached_native_handle_for_runtime(scope, runtime_ptr, target) else {
        return false;
    };
    let values = (1..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    mutate_parent_node(scope, runtime_ptr, parent, mutation, &values, true);
    // Once argument conversion begins, an exception must not fall through to
    // the legacy implementation and repeat conversions or partial mutations.
    true
}

fn detached_replace_children_for_public_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    nodes: Vec<v8::Local<'s, v8::Object>>,
) -> bool {
    let children = detached_child_node_objects(scope, target);
    for child in children {
        detached_detach_from_parent_appending_to_current_reaction_queue(scope, child);
    }
    for node in nodes {
        if !detached_insert_or_throw(scope, target, node, None) {
            return false;
        }
    }
    true
}

pub(in crate::native_bridge) fn bridge_detached_prepend_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    if mutate_native_detached_parent(scope, &args, target, ParentNodeMutation::Prepend) {
        return;
    }
    let document = detached_owner_document_object(scope, target).unwrap_or(target);
    let mut values = Vec::new();
    for index in 1..args.length() {
        values.push(args.get(index));
    }
    let reference = match detached_native_mutation_child_node_objects(scope, target) {
        Some(children) => children.first().copied(),
        None => target
            .get(scope, v8str(scope, "firstChild").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()),
    };
    let Some(nodes) = detached_nodes_from_values(scope, document, &values) else {
        return;
    };
    if !detached_parent_node_pre_insert_validate(scope, target, reference, &nodes) {
        return;
    }
    let Some(fragment) = build_fragment_from_nodes(scope, document, &nodes) else {
        return;
    };
    with_detached_tree_reaction_scope(scope, |scope| {
        let _ = detached_insert_or_throw(scope, target, fragment, reference);
    });
}

fn detached_parent_node_pre_insert_validate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    reference_child: Option<v8::Local<'s, v8::Object>>,
    nodes: &[v8::Local<'s, v8::Object>],
) -> bool {
    if !detached_pre_insert_parent_and_ancestor_are_valid_for_nodes(scope, parent, nodes) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    for node in nodes {
        if !detached_pre_insert_node_type_is_valid(scope, parent, *node) {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return false;
        }
    }
    if detached_node_type(scope, parent) != Some(9) {
        return true;
    }

    let insertion_nodes = nodes
        .iter()
        .flat_map(|node| detached_flattened_insertion_nodes(scope, *node))
        .collect::<Vec<_>>();
    let current_children = detached_native_mutation_child_node_objects(scope, parent)
        .unwrap_or_else(|| detached_child_node_objects(scope, parent))
        .into_iter()
        .filter(|candidate| {
            !insertion_nodes
                .iter()
                .any(|inserted| candidate.strict_equals((*inserted).into()))
        })
        .collect::<Vec<_>>();
    let reference_index = reference_child
        .and_then(|reference_child| {
            current_children
                .iter()
                .position(|candidate| candidate.strict_equals(reference_child.into()))
        })
        .unwrap_or(current_children.len());
    let mut prospective = current_children;
    prospective.splice(reference_index..reference_index, insertion_nodes);
    if !detached_validate_document_children(scope, &prospective) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    true
}

fn detached_pre_insert_parent_and_ancestor_are_valid_for_nodes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    nodes: &[v8::Local<'s, v8::Object>],
) -> bool {
    detached_move_before_valid_parent(scope, parent)
        && nodes
            .iter()
            .all(|node| !detached_shadow_including_contains(scope, *node, parent))
}

pub(in crate::native_bridge) fn bridge_detached_replace_children_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    if mutate_native_detached_parent(scope, &args, target, ParentNodeMutation::ReplaceChildren) {
        return;
    }
    let document = detached_owner_document_object(scope, target).unwrap_or(target);
    let mut values = Vec::new();
    for index in 1..args.length() {
        values.push(args.get(index));
    }
    let mut nodes = Vec::with_capacity(values.len());
    for value in values {
        let node = if value.is_object()
            && let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && detached_node_type(scope, object).is_some()
        {
            object
        } else {
            let Some(text) = value.to_string(scope) else {
                continue;
            };
            let text = text.to_rust_string_lossy(scope);
            let Some(node) = build_detached_text_object(scope, document, &text) else {
                continue;
            };
            node
        };
        nodes.push(node);
    }
    if !detached_parent_node_pre_insert_validate(scope, target, None, &nodes) {
        return;
    }

    let _ = with_detached_tree_reaction_scope(scope, |scope| {
        detached_replace_children_for_public_mutation(scope, target, nodes)
    });
}

fn mutate_native_detached_child<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    target: v8::Local<'s, v8::Object>,
    mutation: ChildNodeMutation,
) -> bool {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, target) else {
        return false;
    };
    let values = (1..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    mutate_child_node(scope, runtime_ptr, handle, mutation, &values, true);
    true
}

fn detached_fragment_from_child_mutation_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut values = Vec::new();
    for index in 1..args.length() {
        values.push(args.get(index));
    }
    build_fragment_from_values(scope, document, &values)
}

pub(in crate::native_bridge) fn bridge_detached_before_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    if mutate_native_detached_child(scope, &args, target, ChildNodeMutation::Before) {
        return;
    }
    let Some(parent) = detached_parent_node_object(scope, target) else {
        return;
    };
    let document = detached_owner_document_object(scope, target).unwrap_or(parent);
    let Some(fragment) = detached_fragment_from_child_mutation_values(scope, document, &args)
    else {
        return;
    };
    with_detached_tree_reaction_scope(scope, |scope| {
        let _ = detached_insert_or_throw(scope, parent, fragment, Some(target));
    });
}

pub(in crate::native_bridge) fn bridge_detached_after_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    if mutate_native_detached_child(scope, &args, target, ChildNodeMutation::After) {
        return;
    }
    let Some(parent) = detached_parent_node_object(scope, target) else {
        return;
    };
    let document = detached_owner_document_object(scope, target).unwrap_or(parent);
    let Some(fragment) = detached_fragment_from_child_mutation_values(scope, document, &args)
    else {
        return;
    };
    let reference = detached_sibling_object(scope, target, 1);
    with_detached_tree_reaction_scope(scope, |scope| {
        let _ = detached_insert_or_throw(scope, parent, fragment, reference);
    });
}

pub(in crate::native_bridge) fn bridge_detached_replace_with_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    if mutate_native_detached_child(scope, &args, target, ChildNodeMutation::ReplaceWith) {
        return;
    }
    let Some(parent) = detached_parent_node_object(scope, target) else {
        return;
    };
    let document = detached_owner_document_object(scope, target).unwrap_or(parent);
    let Some(fragment) = detached_fragment_from_child_mutation_values(scope, document, &args)
    else {
        return;
    };
    with_detached_tree_reaction_scope(scope, |scope| {
        if detached_insert_or_throw(scope, parent, fragment, Some(target)) {
            detached_detach_from_parent_appending_to_current_reaction_queue(scope, target);
        }
    });
}

pub(in crate::native_bridge) fn bridge_detached_append_child_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(parent) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Ok(child) = v8::Local::<v8::Object>::try_from(args.get(1)) else {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        );
        return;
    };
    let result = with_detached_tree_reaction_scope(scope, |scope| {
        detached_insert_node_appending_to_current_reaction_queue(scope, parent, child, None)
    });
    match result {
        Ok(inserted) => rv.set(inserted.into()),
        Err((name, code, message)) => throw_dom_exception(scope, name, code, message),
    }
}

pub(in crate::native_bridge) fn bridge_detached_insert_before_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(parent) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Ok(child) = v8::Local::<v8::Object>::try_from(args.get(1)) else {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        );
        return;
    };
    let reference_child = if args.get(2).is_null_or_undefined() {
        None
    } else {
        v8::Local::<v8::Object>::try_from(args.get(2)).ok()
    };
    let result = with_detached_tree_reaction_scope(scope, |scope| {
        detached_insert_node_appending_to_current_reaction_queue(
            scope,
            parent,
            child,
            reference_child,
        )
    });
    match result {
        Ok(inserted) => rv.set(inserted.into()),
        Err((name, code, message)) => throw_dom_exception(scope, name, code, message),
    }
}

pub(in crate::native_bridge) fn bridge_detached_move_before_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(parent) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    if args.length() < 3 {
        throw_type_error(
            scope,
            "Failed to execute 'moveBefore' on 'Node': 2 arguments required.",
        );
        return;
    }
    let Ok(child) = v8::Local::<v8::Object>::try_from(args.get(1)) else {
        throw_type_error(
            scope,
            "Failed to execute 'moveBefore' on 'Node': parameter 1 is not of type 'Node'.",
        );
        return;
    };
    let reference_child = if args.get(2).is_null_or_undefined() {
        None
    } else {
        let Ok(reference_child) = v8::Local::<v8::Object>::try_from(args.get(2)) else {
            throw_type_error(
                scope,
                "Failed to execute 'moveBefore' on 'Node': parameter 2 is not of type 'Node'.",
            );
            return;
        };
        Some(reference_child)
    };
    if !detached_move_before_valid_parent(scope, parent)
        || !detached_move_before_valid_child(scope, child)
        || detached_shadow_including_contains(scope, child, parent)
        || !detached_shadow_including_roots_match(scope, parent, child)
    {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        );
        return;
    }
    if let Some(reference_child) = reference_child
        && !detached_parent_node_object(scope, reference_child)
            .is_some_and(|value| value.strict_equals(parent.into()))
    {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node before which the node is to be moved is not a child of this node.",
        );
        return;
    }
    if reference_child.is_some_and(|reference_child| reference_child.strict_equals(child.into())) {
        return;
    }
    let result = with_detached_tree_reaction_scope(scope, |scope| {
        detached_insert_node_appending_to_current_reaction_queue(
            scope,
            parent,
            child,
            reference_child,
        )
    });
    if let Err((name, code, message)) = result {
        throw_dom_exception(scope, name, code, message);
    }
}

fn detached_move_before_valid_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(detached_node_type(scope, node), Some(1 | 9 | 11))
}

fn detached_move_before_valid_child<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(detached_node_type(scope, node), Some(1 | 3 | 4 | 7 | 8))
}

fn detached_shadow_including_contains<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    ancestor: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    let mut current = Some(node);
    while let Some(candidate) = current {
        if candidate.strict_equals(ancestor.into()) {
            return true;
        }
        current = detached_parent_node_object(scope, candidate)
            .or_else(|| detached_shadow_host_object(scope, candidate));
    }
    false
}

fn detached_shadow_including_roots_match<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    left: v8::Local<'s, v8::Object>,
    right: v8::Local<'s, v8::Object>,
) -> bool {
    match (
        detached_shadow_including_root(scope, left),
        detached_shadow_including_root(scope, right),
    ) {
        (Some(left), Some(right)) => left.strict_equals(right.into()),
        _ => false,
    }
}

fn detached_shadow_including_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut root = node;
    while let Some(parent) = detached_parent_node_object(scope, root) {
        root = parent;
    }
    while detached_state_kind(scope, root).as_deref() == Some("shadowRoot") {
        root = detached_shadow_host_object(scope, root)?;
        while let Some(parent) = detached_parent_node_object(scope, root) {
            root = parent;
        }
    }
    Some(root)
}

fn detached_shadow_host_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if detached_state_kind(scope, node).as_deref() != Some("shadowRoot") {
        return None;
    }
    detached_state_object(scope, node)
        .and_then(|state| object_property_as_object(scope, state, "host"))
}

pub(in crate::native_bridge) fn bridge_detached_remove_child_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(parent) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Ok(child) = v8::Local::<v8::Object>::try_from(args.get(1)) else {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node to be removed is not a child of this node.",
        );
        return;
    };
    if !detached_parent_node_object(scope, child)
        .is_some_and(|value| value.strict_equals(parent.into()))
    {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node to be removed is not a child of this node.",
        );
        return;
    }
    let owner_document = if detached_node_type(scope, parent) == Some(9) {
        Some(parent)
    } else {
        detached_owner_document_object(scope, parent)
    };
    with_detached_tree_reaction_scope(scope, |scope| {
        detached_detach_from_parent_appending_to_current_reaction_queue(scope, child);
        if let Some(owner_document) = owner_document {
            detached_set_owner_document(scope, child, owner_document);
        }
    });
    rv.set(child.into());
}

pub(in crate::native_bridge) fn bridge_detached_replace_child_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(parent) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(parent_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, parent)
    {
        if replace_node_child(
            scope,
            runtime_ptr,
            parent_handle,
            args.get(1),
            args.get(2),
            true,
        )
        .is_some()
        {
            rv.set(args.get(2));
        }
        return;
    }
    let Ok(new_child) = v8::Local::<v8::Object>::try_from(args.get(1)) else {
        throw_dom_exception(
            scope,
            "HierarchyRequestError",
            3,
            "The operation would yield an invalid node tree.",
        );
        return;
    };
    let Ok(old_child) = v8::Local::<v8::Object>::try_from(args.get(2)) else {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node to be replaced is not a child of this node.",
        );
        return;
    };
    if !detached_pre_insert_parent_and_ancestor_are_valid(scope, parent, new_child) {
        let (name, code, message) = detached_pre_insert_hierarchy_error();
        throw_dom_exception(scope, name, code, message);
        return;
    }
    if detached_document_replace_would_violate_root_shape(scope, parent, new_child, old_child) {
        let (name, code, message) = detached_pre_insert_hierarchy_error();
        throw_dom_exception(scope, name, code, message);
        return;
    }
    let old_child_is_child = detached_parent_node_object(scope, old_child)
        .is_some_and(|value| value.strict_equals(parent.into()))
        || detached_document_direct_element_child_matches(scope, parent, old_child);
    if !old_child_is_child {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The node to be replaced is not a child of this node.",
        );
        return;
    }
    if !detached_pre_insert_node_type_is_valid(scope, parent, new_child) {
        let (name, code, message) = detached_pre_insert_hierarchy_error();
        throw_dom_exception(scope, name, code, message);
        return;
    }
    if new_child.strict_equals(old_child.into()) {
        rv.set(old_child.into());
        return;
    }
    let replaced = with_detached_tree_reaction_scope(scope, |scope| {
        if !detached_insert_or_throw(scope, parent, new_child, Some(old_child)) {
            return false;
        }
        detached_detach_from_parent_appending_to_current_reaction_queue(scope, old_child);
        true
    });
    if !replaced {
        let (name, code, message) = detached_pre_insert_hierarchy_error();
        throw_dom_exception(scope, name, code, message);
        return;
    }
    rv.set(old_child.into());
}
