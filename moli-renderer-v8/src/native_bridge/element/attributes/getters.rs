use crate::dom::native::{Element, Node};
use crate::util::{v8_string, v8_string_from_utf16_units};
use crate::webidl;

use super::super::super::{
    document,
    node::{
        node_runtime_and_handle_from_args_or_detached, require_element_method_receiver,
        throw_incompatible_method_receiver,
    },
};
use super::super::{element_attribute_names, element_has_attribute};
use super::{AttributeNameArgs, AttributeNamespaceNameArgs};

fn element_method_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method: &str,
) -> Option<(
    *mut super::super::super::JsContextHost,
    crate::document_runtime::DomHandle,
)> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, args)
    else {
        throw_incompatible_method_receiver(scope, "Element", method);
        return None;
    };
    if !require_element_method_receiver(scope, unsafe { &*runtime_ptr }, handle, method) {
        return None;
    }
    Some((runtime_ptr, handle))
}

pub(in crate::native_bridge) fn node_has_attribute_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "hasAttribute") else {
        rv.set_bool(false);
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_has_attribute_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<AttributeNameArgs>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        &parsed.name,
    ));
}

pub(in crate::native_bridge) fn node_has_attribute_ns_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "hasAttributeNS")
    else {
        rv.set_bool(false);
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_has_attribute_ns_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<AttributeNamespaceNameArgs>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    let namespace = parsed.namespace.filter(|namespace| !namespace.is_empty());
    rv.set_bool(unsafe { &*runtime_ptr }.dom_host().has_attribute_ns(
        handle,
        namespace.as_deref(),
        &parsed.local_name,
    ));
}

pub(in crate::native_bridge) fn node_has_attributes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "hasAttributes") else {
        rv.set_bool(false);
        return;
    };
    let has_attributes = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(Element::has_attributes)
        .unwrap_or(false);
    rv.set_bool(has_attributes);
}

pub(in crate::native_bridge) fn node_get_attribute_names_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "getAttributeNames")
    else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_get_attribute_names_method_callback(scope, args, rv);
        return;
    }
    let values = element_attribute_names(unsafe { &*runtime_ptr }, handle)
        .into_iter()
        .filter_map(|name| v8_string(scope, &name).map(Into::into))
        .collect::<Vec<v8::Local<'_, v8::Value>>>();
    rv.set(v8::Array::new_with_elements(scope, &values).into());
}

pub(in crate::native_bridge) fn node_get_attribute_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, _)) = element_method_receiver(scope, &args, "getAttribute") else {
        rv.set_null();
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_get_attribute_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<AttributeNameArgs>(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "getAttribute") else {
        return;
    };
    let Some(value) = unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute_utf16_units(handle, &parsed.name)
    else {
        rv.set_null();
        return;
    };
    let Some(value) = v8_string_from_utf16_units(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn node_get_attribute_ns_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, _)) = element_method_receiver(scope, &args, "getAttributeNS") else {
        rv.set_null();
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_get_attribute_ns_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<AttributeNamespaceNameArgs>(scope, &args) else {
        rv.set_null();
        return;
    };
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "getAttributeNS")
    else {
        return;
    };
    let namespace = parsed.namespace.filter(|namespace| !namespace.is_empty());
    let Some(value) = unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute_ns_utf16_units(handle, namespace.as_deref(), &parsed.local_name)
    else {
        rv.set_null();
        return;
    };
    let Some(value) = v8_string_from_utf16_units(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}
