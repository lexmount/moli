use super::fragment::{convert_node_or_string_arguments, convert_nodes_into_node};
use super::*;
use crate::custom_elements;

#[derive(Clone, Copy)]
pub(in crate::native_bridge) enum ParentNodeMutation {
    Append,
    Prepend,
    ReplaceChildren,
}

impl ParentNodeMutation {
    fn name(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Prepend => "prepend",
            Self::ReplaceChildren => "replaceChildren",
        }
    }
}

pub(in crate::native_bridge) fn node_append_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    parent_node_callback(scope, &args, ParentNodeMutation::Append);
}

pub(in crate::native_bridge) fn node_prepend_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    parent_node_callback(scope, &args, ParentNodeMutation::Prepend);
}

pub(in crate::native_bridge) fn node_replace_children_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    parent_node_callback(scope, &args, ParentNodeMutation::ReplaceChildren);
}

fn parent_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    mutation: ParentNodeMutation,
) {
    let Ok((runtime_ptr, parent)) = node_runtime_and_handle_from_args_or_detached(scope, args)
    else {
        throw_incompatible_method_receiver(scope, "ParentNode", mutation.name());
        return;
    };
    let values = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    mutate_parent_node(scope, runtime_ptr, parent, mutation, &values, false);
}

pub(in crate::native_bridge) fn mutate_parent_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    parent: DomHandle,
    mutation: ParentNodeMutation,
    values: &[v8::Local<'s, v8::Value>],
    detached: bool,
) {
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        parent,
        mutation.name(),
        true,
    ) {
        return;
    }
    let Some(inputs) = convert_node_or_string_arguments(scope, values) else {
        return;
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let document = insertion_document_handle(unsafe { &*runtime_ptr }, parent);
        let Some(node) = convert_nodes_into_node(scope, runtime_ptr, document, inputs) else {
            return;
        };
        // Conversion can move existing children. Read the reference and the
        // children excluded by replaceChildren only after conversion completes.
        let runtime = unsafe { &*runtime_ptr };
        let reference = match mutation {
            ParentNodeMutation::Prepend => runtime.dom_host().child_handles(parent).next(),
            _ => None,
        };
        let excluded = match mutation {
            ParentNodeMutation::ReplaceChildren => {
                runtime.dom_host().child_handles(parent).collect()
            }
            _ => Vec::new(),
        };
        if !validate_pre_insert_handles(scope, runtime, parent, node, reference, &excluded) {
            return;
        }
        let runtime = unsafe { &mut *runtime_ptr };
        let inserted = match mutation {
            ParentNodeMutation::ReplaceChildren => runtime
                .replace_all_children_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    Some(node),
                    detached,
                ),
            _ if detached => runtime
                .insert_detached_native_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    node,
                    reference,
                ),
            _ => runtime.insert_before_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                node,
                reference,
            ),
        };
        if !inserted {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        }
    });
}
