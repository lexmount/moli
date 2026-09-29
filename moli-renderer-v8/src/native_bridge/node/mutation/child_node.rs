use super::fragment::{NodeOrString, convert_node_or_string_arguments, convert_nodes_into_node};
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

#[derive(Clone, Copy)]
pub(in crate::native_bridge) enum ChildNodeMutation {
    Before,
    After,
    ReplaceWith,
}

impl ChildNodeMutation {
    fn name(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
            Self::ReplaceWith => "replaceWith",
        }
    }
}

pub(in crate::native_bridge) fn node_before_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    child_node_callback(scope, &args, ChildNodeMutation::Before);
}

pub(in crate::native_bridge) fn node_after_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    child_node_callback(scope, &args, ChildNodeMutation::After);
}

pub(in crate::native_bridge) fn node_replace_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    child_node_callback(scope, &args, ChildNodeMutation::ReplaceWith);
}

fn child_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    mutation: ChildNodeMutation,
) {
    let Ok((runtime_ptr, target)) = node_runtime_and_handle_from_args_or_detached(scope, args)
    else {
        throw_incompatible_method_receiver(scope, "ChildNode", mutation.name());
        return;
    };
    let values = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    mutate_child_node(scope, runtime_ptr, target, mutation, &values, false);
}

pub(in crate::native_bridge) fn mutate_child_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    mutation: ChildNodeMutation,
    values: &[v8::Local<'s, v8::Value>],
    detached: bool,
) {
    if !require_child_node_receiver(scope, unsafe { &*runtime_ptr }, target, mutation.name()) {
        return;
    }
    let Some(inputs) = convert_node_or_string_arguments(scope, values) else {
        return;
    };
    // Web IDL conversion runs even for a parentless receiver, and author
    // toString hooks can change its parent before the DOM algorithm begins.
    let Some(parent) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(target)
        .and_then(Node::parent_node)
    else {
        return;
    };
    let input_handles = inputs
        .iter()
        .filter_map(|input| match input {
            NodeOrString::Node(value) => {
                node_or_existing_detached_arg_handle(scope, runtime_ptr, *value)
            }
            NodeOrString::String(_) => None,
        })
        .collect::<Vec<_>>();
    let runtime = unsafe { &*runtime_ptr };
    let sibling = |handle| {
        runtime
            .dom_host()
            .node(handle)
            .and_then(|node| match mutation {
                ChildNodeMutation::Before => node.prev_sibling(),
                _ => node.next_sibling(),
            })
    };
    let mut viable_sibling = sibling(target);
    while viable_sibling.is_some_and(|handle| input_handles.contains(&handle)) {
        viable_sibling = viable_sibling.and_then(sibling);
    }
    let document = insertion_document_handle(runtime, target);
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let Some(node) = convert_nodes_into_node(scope, runtime_ptr, document, inputs) else {
            return;
        };
        let runtime = unsafe { &*runtime_ptr };
        let replaces_target = matches!(mutation, ChildNodeMutation::ReplaceWith)
            && runtime.dom_host().node(target).and_then(Node::parent_node) == Some(parent);
        let reference = if replaces_target {
            Some(target)
        } else if matches!(mutation, ChildNodeMutation::Before) {
            match viable_sibling {
                Some(previous) => runtime
                    .dom_host()
                    .node(previous)
                    .and_then(Node::next_sibling),
                None => runtime.dom_host().child_handles(parent).next(),
            }
        } else {
            viable_sibling
        };
        let excluded = if replaces_target {
            std::slice::from_ref(&target)
        } else {
            &[]
        };
        if !validate_pre_insert_handles(scope, runtime, parent, node, reference, excluded) {
            return;
        }
        let runtime = unsafe { &mut *runtime_ptr };
        let changed = match (replaces_target, detached) {
            (true, true) => runtime
                .replace_detached_native_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    node,
                    target,
                ),
            (true, false) => runtime.replace_child_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                node,
                target,
            ),
            (false, true) => runtime
                .insert_detached_native_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    node,
                    reference,
                ),
            (false, false) => runtime.insert_before_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                node,
                reference,
            ),
        };
        if !changed {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        }
    });
}
