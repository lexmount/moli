use super::core::{
    node_can_contain_children, node_move_before_shadow_including_contains, node_type_is_insertable,
    pre_insert_validation_handles,
};
use super::fragment::{
    build_insertion_fragment_from_handles, insert_fragment_with_validation,
    validate_document_sequence, value_to_inserted_handle,
};
use super::*;
use crate::custom_elements;

pub(in crate::native_bridge) fn node_append_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "ParentNode", "append");
        return;
    };
    if !require_parent_node_receiver(scope, unsafe { &*runtime_ptr }, parent, "append", true) {
        return;
    }
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(inserted) = parent_node_insertion_handles(scope, runtime_ptr, document_handle, &args)
    else {
        return;
    };
    if !validate_parent_node_insertion(scope, unsafe { &*runtime_ptr }, parent, None, &inserted) {
        return;
    }
    let Some(appended) =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let fragment = build_insertion_fragment_from_handles(
                scope,
                runtime_ptr,
                document_handle,
                &inserted,
            )?;
            Some(insert_fragment_with_validation(
                scope,
                runtime_ptr,
                parent,
                None,
                &[],
                fragment,
                &inserted,
            ))
        })
    else {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    };
    if !appended {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
    }
}

pub(in crate::native_bridge) fn node_prepend_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "ParentNode", "prepend");
        return;
    };
    if !require_parent_node_receiver(scope, unsafe { &*runtime_ptr }, parent, "prepend", true) {
        return;
    }
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(inserted) = parent_node_insertion_handles(scope, runtime_ptr, document_handle, &args)
    else {
        return;
    };
    // Converting the arguments into a node moves existing children into the
    // temporary fragment before the pre-insert step. The reference is thus
    // the first child that is not itself among the converted arguments.
    let reference_child = {
        let runtime = unsafe { &*runtime_ptr };
        runtime.dom_host().node(parent).and_then(|node| {
            node.child_ids(runtime.dom_host().dom())
                .find(|child| !inserted.contains(child))
        })
    };
    if !validate_parent_node_insertion(
        scope,
        unsafe { &*runtime_ptr },
        parent,
        reference_child,
        &inserted,
    ) {
        return;
    }
    let Some(prepended) =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let fragment = build_insertion_fragment_from_handles(
                scope,
                runtime_ptr,
                document_handle,
                &inserted,
            )?;
            Some(insert_fragment_with_validation(
                scope,
                runtime_ptr,
                parent,
                reference_child,
                &[],
                fragment,
                &inserted,
            ))
        })
    else {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    };
    if !prepended {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
    }
}

fn parent_node_insertion_handles(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: Option<DomHandle>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<Vec<DomHandle>> {
    let mut inserted = Vec::new();
    for index in 0..args.length() {
        let handle =
            value_to_inserted_handle(scope, runtime_ptr, document_handle, args.get(index))?;
        inserted.push(handle);
    }
    Some(inserted)
}

fn validate_parent_node_insertion(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    parent: DomHandle,
    reference_child: Option<DomHandle>,
    inserted: &[DomHandle],
) -> bool {
    if !node_can_contain_children(runtime, parent) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    for child in inserted {
        if node_move_before_shadow_including_contains(runtime, *child, parent) {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return false;
        }
        let Some(child_node_type) = runtime.dom_host().node(*child).map(Node::node_type) else {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return false;
        };
        if !node_type_is_insertable(child_node_type)
            || (child_node_type == NodeType::Text && node_is_document(runtime, parent))
            || (child_node_type == NodeType::DocumentType && !node_is_document(runtime, parent))
        {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return false;
        }
    }
    let document_inserted = inserted
        .iter()
        .flat_map(|handle| pre_insert_validation_handles(runtime, *handle))
        .collect::<Vec<_>>();
    if !validate_document_sequence(
        runtime,
        parent,
        reference_child,
        &document_inserted,
        inserted,
    ) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    true
}

pub(in crate::native_bridge) fn node_replace_children_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "ParentNode", "replaceChildren");
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        parent,
        "replaceChildren",
        true,
    ) {
        return;
    }
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let mut inserted = Vec::new();
    for index in 0..args.length() {
        let Some(handle) =
            value_to_inserted_handle(scope, runtime_ptr, document_handle, args.get(index))
        else {
            return;
        };
        inserted.push(handle);
    }
    let existing_children = unsafe { &*runtime_ptr }
        .dom_host()
        .node(parent)
        .map(|node| {
            node.child_ids(unsafe { &*runtime_ptr }.dom_host().dom())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !validate_parent_node_replace_children(scope, unsafe { &*runtime_ptr }, parent, &inserted) {
        return;
    }
    let Some(()) =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let fragment = build_insertion_fragment_from_handles(
                scope,
                runtime_ptr,
                document_handle,
                &inserted,
            )?;
            let runtime = unsafe { &mut *runtime_ptr };
            runtime.replace_all_children_with_fragment_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                fragment,
                &existing_children,
            );
            Some(())
        })
    else {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    };
}

fn validate_parent_node_replace_children(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    parent: DomHandle,
    inserted: &[DomHandle],
) -> bool {
    if !node_can_contain_children(runtime, parent) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    for child in inserted {
        if node_move_before_shadow_including_contains(runtime, *child, parent) {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return false;
        }
        let Some(child_node_type) = runtime.dom_host().node(*child).map(Node::node_type) else {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return false;
        };
        if !node_type_is_insertable(child_node_type)
            || (child_node_type == NodeType::Text && node_is_document(runtime, parent))
            || (child_node_type == NodeType::DocumentType && !node_is_document(runtime, parent))
        {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return false;
        }
    }
    let document_inserted = inserted
        .iter()
        .flat_map(|handle| pre_insert_validation_handles(runtime, *handle))
        .collect::<Vec<_>>();
    if !validate_document_sequence(runtime, parent, None, &document_inserted, inserted) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return false;
    }
    true
}
