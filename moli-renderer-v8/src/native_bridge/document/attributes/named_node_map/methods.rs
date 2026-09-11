use crate::webidl;

use super::super::attribute_node::{live_remove_attribute_node, live_set_attribute_node};
use super::helpers::{
    attribute_node_for_index, attribute_node_for_name, attribute_node_for_namespace,
    named_node_map_element,
};
use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NamedNodeMap.item")]
struct NamedNodeMapItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NamedNodeMap.getNamedItem")]
struct NamedNodeMapNameArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NamedNodeMap.getNamedItemNS")]
struct NamedNodeMapNamespaceArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    local_name: String,
}

pub(in crate::native_bridge::document) fn named_node_map_item_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(element) = named_node_map_element(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<NamedNodeMapItemArgs>(scope, &args) else {
        return;
    };
    let Some(value) = attribute_node_for_index(scope, element, parsed.index as usize) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(in crate::native_bridge::document) fn named_node_map_get_named_item_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(element) = named_node_map_element(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<NamedNodeMapNameArgs>(scope, &args) else {
        return;
    };
    let Some(value) = attribute_node_for_name(scope, element, &parsed.name) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(in crate::native_bridge::document) fn named_node_map_set_named_item_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(element) = named_node_map_element(scope, args.this()) else {
        rv.set_null();
        return;
    };
    if let Some((runtime_ptr, _)) = detached_native_element_runtime_and_handle(scope, element) {
        crate::custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if let Some(value) = detached_native_set_attribute_node(scope, element, args.get(0)) {
                rv.set(value);
            }
        });
        return;
    }
    let Some(value) = live_set_attribute_node(scope, element, args.get(0)) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(in crate::native_bridge::document) fn named_node_map_remove_named_item_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(element) = named_node_map_element(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<NamedNodeMapNameArgs>(scope, &args) else {
        return;
    };
    let Some(attr) = attribute_node_for_name(scope, element, &parsed.name) else {
        throw_dom_exception(scope, "NotFoundError", 8, "The attribute was not found.");
        return;
    };
    if attr.is_null_or_undefined() {
        throw_dom_exception(scope, "NotFoundError", 8, "The attribute was not found.");
        return;
    }
    if let Some((runtime_ptr, _)) = detached_native_element_runtime_and_handle(scope, element) {
        let mut handled = false;
        crate::custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if let Some(value) = detached_native_remove_attribute_node(scope, element, attr) {
                rv.set(value);
                handled = true;
            }
        });
        if handled {
            return;
        }
    }
    if let Some(value) = detached_native_remove_attribute_node(scope, element, attr) {
        rv.set(value);
        return;
    }
    let Some(value) = live_remove_attribute_node(scope, element, attr) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(in crate::native_bridge::document) fn named_node_map_get_named_item_ns_method_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(element) = named_node_map_element(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<NamedNodeMapNamespaceArgs>(scope, &args) else {
        return;
    };
    let namespace = parsed
        .namespace
        .as_deref()
        .filter(|namespace| !namespace.is_empty());
    let Some(value) = attribute_node_for_namespace(scope, element, namespace, &parsed.local_name)
    else {
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(in crate::native_bridge::document) fn named_node_map_set_named_item_ns_method_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(element) = named_node_map_element(scope, args.this()) else {
        rv.set_null();
        return;
    };
    if let Some((runtime_ptr, _)) = detached_native_element_runtime_and_handle(scope, element) {
        crate::custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if let Some(value) = detached_native_set_attribute_node(scope, element, args.get(0)) {
                rv.set(value);
            }
        });
        return;
    }
    let Some(value) = live_set_attribute_node(scope, element, args.get(0)) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(in crate::native_bridge::document) fn named_node_map_remove_named_item_ns_method_callback<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(element) = named_node_map_element(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<NamedNodeMapNamespaceArgs>(scope, &args) else {
        return;
    };
    let namespace = parsed
        .namespace
        .as_deref()
        .filter(|namespace| !namespace.is_empty());
    let Some(attr) = attribute_node_for_namespace(scope, element, namespace, &parsed.local_name)
    else {
        throw_dom_exception(scope, "NotFoundError", 8, "The attribute was not found.");
        return;
    };
    if attr.is_null_or_undefined() {
        throw_dom_exception(scope, "NotFoundError", 8, "The attribute was not found.");
        return;
    }
    if let Some((runtime_ptr, _)) = detached_native_element_runtime_and_handle(scope, element) {
        let mut handled = false;
        crate::custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            if let Some(value) = detached_native_remove_attribute_node(scope, element, attr) {
                rv.set(value);
                handled = true;
            }
        });
        if handled {
            return;
        }
    }
    if let Some(value) = detached_native_remove_attribute_node(scope, element, attr) {
        rv.set(value);
        return;
    }
    let Some(value) = live_remove_attribute_node(scope, element, attr) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}
