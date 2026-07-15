use crate::custom_elements;
use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::webidl;

use super::super::super::{
    JsContextHost, document,
    node::{
        node_runtime_and_handle_from_args_or_detached, require_element_method_receiver,
        throw_incompatible_method_receiver,
    },
    throw_dom_exception, validate_attribute_name, validate_qualified_name_and_namespace,
};
use super::super::{
    TrustedAttributeSetter, element_has_attribute,
    remove_live_element_attribute_appending_to_current_reaction_queue,
    remove_live_element_attribute_ns_appending_to_current_reaction_queue,
    set_live_element_attribute_appending_to_current_reaction_queue,
    set_live_element_attribute_ns_appending_to_current_reaction_queue,
    trusted_attribute_value_string, update_iframe_snapshot_navigation,
};
use super::{
    AttributeNameArgs, AttributeNamespaceNameArgs, SetAttributeArgs, SetAttributeNsArgs,
    ToggleAttributeArgs,
};

fn throw_invalid_attribute_name(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidCharacterError",
        5,
        "The attribute name is not valid.",
    );
}

fn element_method_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method: &str,
) -> Option<(*mut JsContextHost, DomHandle)> {
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

pub(super) fn attribute_target_for_remove_name(
    runtime: &JsContextHost,
    handle: DomHandle,
    name: &str,
) -> Option<(Option<String>, String)> {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    let normalized_name = runtime.dom_host().normalized_attribute_name(handle, name)?;
    element
        .attributes()
        .iter()
        .find(|attribute| attribute.name_matches(&normalized_name))
        .map(|attribute| {
            (
                (!attribute.namespace().is_empty()).then(|| attribute.namespace().to_owned()),
                attribute.local_name().to_owned(),
            )
        })
}

pub(in crate::native_bridge) fn node_set_attribute_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "setAttribute") else {
        rv.set_undefined();
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_set_attribute_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<SetAttributeArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    if !validate_attribute_name(&parsed.name) {
        throw_invalid_attribute_name(scope);
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    let normalized_name = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .map(|element| element.normalized_attribute_name(&parsed.name))
        .unwrap_or_else(|| parsed.name.clone());
    let Some(value) = trusted_attribute_value_string(
        scope,
        Some((runtime_ptr, handle)),
        None,
        &normalized_name,
        parsed.value,
        TrustedAttributeSetter::SetAttribute,
    ) else {
        rv.set_undefined();
        return;
    };
    if parsed.name.eq_ignore_ascii_case("src")
        && runtime.dom_host().is_html_element_named(handle, "iframe")
    {
        update_iframe_snapshot_navigation(scope, runtime_ptr, handle, &value);
        rv.set_undefined();
        return;
    }
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = set_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            &parsed.name,
            &value,
        );
    });
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_set_attribute_ns_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "setAttributeNS")
    else {
        rv.set_undefined();
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_set_attribute_ns_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<SetAttributeNsArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let namespace = parsed.namespace.filter(|namespace| !namespace.is_empty());
    let (prefix, local_name) =
        match validate_qualified_name_and_namespace(namespace.as_deref(), &parsed.qualified_name) {
            Ok(parts) => parts,
            Err((name, code, message)) => {
                throw_dom_exception(scope, name, code, message);
                return;
            }
        };
    let Some(value) = trusted_attribute_value_string(
        scope,
        Some((runtime_ptr, handle)),
        namespace.as_deref(),
        &local_name,
        parsed.value,
        TrustedAttributeSetter::SetAttributeNs,
    ) else {
        rv.set_undefined();
        return;
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = set_live_element_attribute_ns_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            namespace.as_deref(),
            prefix.as_deref(),
            &local_name,
            &parsed.qualified_name,
            &value,
        );
    });
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_remove_attribute_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "removeAttribute")
    else {
        rv.set_undefined();
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_remove_attribute_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<AttributeNameArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    if let Some((namespace, local_name)) =
        attribute_target_for_remove_name(unsafe { &*runtime_ptr }, handle, &parsed.name)
    {
        super::super::super::document::clear_live_attr_cache_entry_ns(
            scope,
            args.this(),
            namespace.as_deref(),
            &local_name,
        );
    }
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = remove_live_element_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            &parsed.name,
        );
    });
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_remove_attribute_ns_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "removeAttributeNS")
    else {
        rv.set_undefined();
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_remove_attribute_ns_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<AttributeNamespaceNameArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let namespace = parsed.namespace.filter(|namespace| !namespace.is_empty());
    super::super::super::document::clear_live_attr_cache_entry_ns(
        scope,
        args.this(),
        namespace.as_deref(),
        &parsed.local_name,
    );
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = remove_live_element_attribute_ns_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            namespace.as_deref(),
            &parsed.local_name,
        );
    });
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_toggle_attribute_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = element_method_receiver(scope, &args, "toggleAttribute")
    else {
        rv.set_bool(false);
        return;
    };
    let Some(parsed) = webidl::parse_args::<ToggleAttributeArgs>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    if !validate_attribute_name(&parsed.name) {
        throw_invalid_attribute_name(scope);
        return;
    }
    let has_attribute = element_has_attribute(unsafe { &*runtime_ptr }, handle, &parsed.name);
    let next_value = parsed.force.unwrap_or(!has_attribute);
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_boolean_attribute_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            &parsed.name,
            next_value,
        );
    });
    rv.set_bool(next_value);
}
