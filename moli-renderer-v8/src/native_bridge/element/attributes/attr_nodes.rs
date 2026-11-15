use crate::util::call_global_bridge_method;
use crate::{web_api_interfaces, webidl};

use super::super::super::{
    document,
    node::{
        node_runtime_and_handle_from_args_or_detached, require_element_method_receiver,
        throw_incompatible_method_receiver,
    },
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.getAttributeNode")]
struct ElementGetAttributeNodeArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.getAttributeNodeNS")]
struct ElementGetAttributeNodeNsArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    local_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.setAttributeNode")]
struct ElementSetAttributeNodeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Attr)]
    attr: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.removeAttributeNode")]
struct ElementRemoveAttributeNodeArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Attr)]
    attr: v8::Local<'s, v8::Object>,
}

pub(in crate::native_bridge) fn node_get_attribute_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "getAttributeNode");
        rv.set_null();
        return;
    };
    if !require_element_method_receiver(scope, unsafe { &*runtime_ptr }, handle, "getAttributeNode")
    {
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_get_attribute_node_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<ElementGetAttributeNodeArgs>(scope, &args) else {
        return;
    };
    let element = args.this();
    match super::super::super::document::live_get_attribute_node_object(
        scope,
        element,
        &parsed.name,
    ) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_get_attribute_node_ns_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "getAttributeNodeNS");
        rv.set_null();
        return;
    };
    if !require_element_method_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "getAttributeNodeNS",
    ) {
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_get_attribute_node_ns_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<ElementGetAttributeNodeNsArgs>(scope, &args) else {
        return;
    };
    let namespace = parsed
        .namespace
        .as_deref()
        .filter(|namespace| !namespace.is_empty());
    let element = args.this();
    match super::super::super::document::live_get_attribute_node_ns_object(
        scope,
        element,
        namespace,
        &parsed.local_name,
    ) {
        Some(attr) => rv.set(attr.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_set_attribute_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "setAttributeNode");
        rv.set_null();
        return;
    };
    if !require_element_method_receiver(scope, unsafe { &*runtime_ptr }, handle, "setAttributeNode")
    {
        return;
    };
    let Some(parsed) = webidl::parse_args::<ElementSetAttributeNodeArgs>(scope, &args) else {
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_set_attribute_node_method_callback(scope, args, rv);
        return;
    }
    let this_value: v8::Local<'_, v8::Value> = args.this().into();
    let this_value = v8::Global::new(scope, this_value);
    let this_value = v8::Local::new(scope, &this_value);
    let attr_value: v8::Local<'_, v8::Value> = parsed.attr.into();
    let attr_value = v8::Global::new(scope, attr_value);
    let attr_value = v8::Local::new(scope, &attr_value);
    match call_global_bridge_method(
        scope,
        "__setAttributeNodeForLiveElement",
        &[this_value, attr_value],
    ) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_remove_attribute_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "removeAttributeNode");
        rv.set_null();
        return;
    };
    if !require_element_method_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "removeAttributeNode",
    ) {
        return;
    };
    let Some(parsed) = webidl::parse_args::<ElementRemoveAttributeNodeArgs>(scope, &args) else {
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_remove_attribute_node_method_callback(scope, args, rv);
        return;
    }
    let this_value: v8::Local<'_, v8::Value> = args.this().into();
    let this_value = v8::Global::new(scope, this_value);
    let this_value = v8::Local::new(scope, &this_value);
    let attr_value: v8::Local<'_, v8::Value> = parsed.attr.into();
    let attr_value = v8::Global::new(scope, attr_value);
    let attr_value = v8::Local::new(scope, &attr_value);
    match call_global_bridge_method(
        scope,
        "__removeAttributeNodeForLiveElement",
        &[this_value, attr_value],
    ) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}
