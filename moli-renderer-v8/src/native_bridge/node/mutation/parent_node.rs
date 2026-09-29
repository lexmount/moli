use super::*;
use crate::{custom_elements, util::v8_value_to_dom_string_u16};
use widestring::U16String;

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

enum NodeOrString<'s> {
    Node(v8::Local<'s, v8::Value>),
    String(U16String),
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
    // Web IDL converts every (Node or DOMString) argument before the DOM
    // algorithm runs, including strings after a node that will fail insertion.
    let mut inputs = Vec::with_capacity(values.len());
    for &value in values {
        if v8::Local::<v8::Object>::try_from(value)
            .is_ok_and(|object| web_api_interfaces::Node::is_instance(scope, object))
        {
            inputs.push(NodeOrString::Node(value));
        } else {
            let Some(value) = v8_value_to_dom_string_u16(scope, value, false) else {
                return;
            };
            inputs.push(NodeOrString::String(value));
        }
    }
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
                .replace_all_children_with_node_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    parent,
                    node,
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

fn converted_input_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: Option<DomHandle>,
    input: NodeOrString<'_>,
) -> Option<DomHandle> {
    match input {
        NodeOrString::Node(value) => {
            if !crate::native_bridge::document::is_attr_node_value(scope, value)
                && let Some(handle) =
                    node_or_foreign_arg_handle_allow_detached(scope, runtime_ptr, document, value)
            {
                return Some(handle);
            }
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            None
        }
        NodeOrString::String(value) => {
            Some(unsafe { &mut *runtime_ptr }.create_text_node_from_utf16_units(document, value))
        }
    }
}

fn convert_nodes_into_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: Option<DomHandle>,
    mut inputs: Vec<NodeOrString<'_>>,
) -> Option<DomHandle> {
    if inputs.len() == 1 {
        return converted_input_handle(scope, runtime_ptr, document, inputs.pop()?);
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let fragment = match document {
        Some(document) => runtime.create_document_fragment_for_document(document),
        None => runtime.create_document_fragment(),
    };
    for input in inputs {
        let child = converted_input_handle(scope, runtime_ptr, document, input)?;
        if !validate_pre_insert_handles(scope, unsafe { &*runtime_ptr }, fragment, child, None, &[])
        {
            return None;
        }
        if !unsafe { &mut *runtime_ptr }.append_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            fragment,
            child,
        ) {
            throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
            return None;
        }
    }
    Some(fragment)
}
