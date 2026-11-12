use super::fragment::{
    build_insertion_fragment_from_handles, insert_fragment_with_validation,
    value_to_inserted_handle,
};
use super::*;
use crate::custom_elements;

pub(in crate::native_bridge) fn node_remove_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "ChildNode", "remove");
        return;
    };
    if !require_child_node_receiver(scope, unsafe { &*runtime_ptr }, handle, "remove") {
        return;
    }
    let Some(parent) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)
    else {
        return;
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.remove_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            parent,
            handle,
        );
    });
}

pub(in crate::native_bridge) fn node_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "ChildNode", "before");
        return;
    };
    if !require_child_node_receiver(scope, unsafe { &*runtime_ptr }, handle, "before") {
        return;
    }
    let Some(parent) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)
    else {
        return;
    };
    let mut values = Vec::new();
    for index in 0..args.length() {
        let value = args.get(index);
        if node_arg_handle(scope, runtime_ptr, value) == Some(handle) {
            continue;
        }
        values.push(value);
    }
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(inserted_handles) =
        child_node_insertion_handles(scope, runtime_ptr, document_handle, values)
    else {
        return;
    };
    let Some(inserted) =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let fragment = build_insertion_fragment_from_handles(
                scope,
                runtime_ptr,
                document_handle,
                &inserted_handles,
            )?;
            Some(insert_fragment_with_validation(
                scope,
                runtime_ptr,
                parent,
                Some(handle),
                &[],
                fragment,
                &inserted_handles,
            ))
        })
    else {
        return;
    };
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
    }
}

pub(in crate::native_bridge) fn node_after_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "ChildNode", "after");
        return;
    };
    if !require_child_node_receiver(scope, unsafe { &*runtime_ptr }, handle, "after") {
        return;
    }
    let Some(parent) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)
    else {
        return;
    };
    let argument_handles = (0..args.length())
        .filter_map(|index| node_arg_handle(scope, runtime_ptr, args.get(index)))
        .collect::<Vec<_>>();
    let mut reference_child = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::next_sibling);
    while reference_child.is_some_and(|sibling| argument_handles.contains(&sibling)) {
        reference_child = reference_child.and_then(|sibling| {
            unsafe { &*runtime_ptr }
                .dom_host()
                .node(sibling)
                .and_then(Node::next_sibling)
        });
    }
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(inserted_handles) = child_node_insertion_handles(
        scope,
        runtime_ptr,
        document_handle,
        (0..args.length()).map(|index| args.get(index)),
    ) else {
        return;
    };
    let Some(inserted) =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let fragment = build_insertion_fragment_from_handles(
                scope,
                runtime_ptr,
                document_handle,
                &inserted_handles,
            )?;
            Some(insert_fragment_with_validation(
                scope,
                runtime_ptr,
                parent,
                reference_child,
                &[],
                fragment,
                &inserted_handles,
            ))
        })
    else {
        return;
    };
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
    }
}

pub(in crate::native_bridge) fn node_replace_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "ChildNode", "replaceWith");
        return;
    };
    if !require_child_node_receiver(scope, unsafe { &*runtime_ptr }, handle, "replaceWith") {
        return;
    }
    let Some(parent) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)
    else {
        return;
    };
    let argument_handles = (0..args.length())
        .filter_map(|index| node_arg_handle(scope, runtime_ptr, args.get(index)))
        .collect::<Vec<_>>();
    let mut reference_child = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::next_sibling);
    while reference_child.is_some_and(|sibling| argument_handles.contains(&sibling)) {
        reference_child = reference_child.and_then(|sibling| {
            unsafe { &*runtime_ptr }
                .dom_host()
                .node(sibling)
                .and_then(Node::next_sibling)
        });
    }
    let document_handle = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
    let Some(inserted_handles) = child_node_insertion_handles(
        scope,
        runtime_ptr,
        document_handle,
        (0..args.length()).map(|index| args.get(index)),
    ) else {
        return;
    };
    let Some(replaced) =
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let Some(fragment) = build_insertion_fragment_from_handles(
                scope,
                runtime_ptr,
                document_handle,
                &inserted_handles,
            ) else {
                let runtime = unsafe { &mut *runtime_ptr };
                let _ = runtime.remove_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    handle,
                );
                return None;
            };
            let added_children = unsafe { &*runtime_ptr }
                .dom_host()
                .child_handles(fragment)
                .collect::<Vec<_>>();
            let records_enabled = unsafe { &*runtime_ptr }
                .dom_host()
                .mutation_records_enabled();
            if !insert_fragment_with_validation(
                scope,
                runtime_ptr,
                parent,
                reference_child,
                &[handle],
                fragment,
                &inserted_handles,
            ) {
                return Some(false);
            }
            let removes_replaced_child = !inserted_handles.contains(&handle);
            if removes_replaced_child {
                let runtime = unsafe { &mut *runtime_ptr };
                let _ = runtime.remove_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    handle,
                );
            }
            if removes_replaced_child && records_enabled && !added_children.is_empty() {
                let previous_sibling = added_children.first().and_then(|child| {
                    unsafe { &*runtime_ptr }
                        .dom_host()
                        .node(*child)
                        .and_then(Node::prev_sibling)
                });
                let next_sibling = added_children.last().and_then(|child| {
                    unsafe { &*runtime_ptr }
                        .dom_host()
                        .node(*child)
                        .and_then(Node::next_sibling)
                });
                crate::observer_runtime::coalesce_child_list_replacement_records(
                    runtime_ptr,
                    parent,
                    &added_children,
                    std::slice::from_ref(&handle),
                    previous_sibling,
                    next_sibling,
                );
            }
            Some(true)
        })
    else {
        return;
    };
    if !replaced {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
    }
}

fn child_node_insertion_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    document_handle: Option<DomHandle>,
    values: impl IntoIterator<Item = v8::Local<'s, v8::Value>>,
) -> Option<Vec<DomHandle>> {
    let mut inserted = Vec::new();
    for value in values {
        let handle = value_to_inserted_handle(scope, runtime_ptr, document_handle, value)?;
        inserted.push(handle);
    }
    Some(inserted)
}
