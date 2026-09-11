use super::*;
use crate::web_api_interfaces;

pub(in crate::native_bridge) fn node_contains_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "contains");
        rv.set_bool(false);
        return;
    };
    let Some(other) = node_or_existing_detached_arg_handle(scope, runtime_ptr, args.get(0)) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(
        unsafe { &*runtime_ptr }
            .dom_host()
            .node(handle)
            .is_some_and(|node| node.contains(unsafe { &*runtime_ptr }.dom_host().dom(), other)),
    );
}

pub(in crate::native_bridge) fn node_has_child_nodes_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "hasChildNodes");
        rv.set_bool(false);
        return;
    };
    let has_children = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .is_some_and(Node::has_child_nodes);
    rv.set_bool(has_children);
}

pub(in crate::native_bridge) fn node_is_same_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "isSameNode");
        rv.set_bool(false);
        return;
    };
    rv.set_bool(
        node_or_existing_detached_arg_handle(scope, runtime_ptr, args.get(0)) == Some(handle),
    );
}

pub(in crate::native_bridge) fn node_is_equal_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "isEqualNode");
        rv.set_bool(false);
        return;
    };
    let Some(other_handle) = node_or_existing_detached_arg_handle(scope, runtime_ptr, args.get(0))
    else {
        rv.set_bool(false);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let equal = match (
        runtime.dom_host().node(handle),
        runtime.dom_host().node(other_handle),
    ) {
        (Some(left), Some(right)) => left.is_equal_node(runtime.dom_host().dom(), right),
        _ => false,
    };
    rv.set_bool(equal);
}

pub(in crate::native_bridge) fn node_compare_document_position_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // DOM spec compareDocumentPosition constants (PRECEDING / FOLLOWING are
    // chosen inside disconnected_order_bit).
    const DOCUMENT_POSITION_DISCONNECTED: u32 = 0x01;
    const DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC: u32 = 0x20;

    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "compareDocumentPosition");
        rv.set(v8::Integer::new_from_unsigned(scope, 0).into());
        return;
    };

    // Fast path: argument is a Node attached to the same runtime tree.
    if let Some(other_handle) =
        node_or_existing_detached_arg_handle(scope, runtime_ptr, args.get(0))
    {
        let runtime = unsafe { &*runtime_ptr };
        let relation = match (
            runtime.dom_host().node(handle),
            runtime.dom_host().node(other_handle),
        ) {
            (Some(left), Some(_)) => {
                left.compare_document_position(runtime.dom_host().dom(), other_handle)
            }
            _ => 0,
        };
        rv.set(v8::Integer::new_from_unsigned(scope, relation as u32).into());
        return;
    }

    // A native Node from another realm or detached document still compares as
    // disconnected even when this runtime cannot resolve its live tree handle.
    let other_value = args.get(0);
    if let Ok(other_object) = v8::Local::<v8::Object>::try_from(other_value)
        && web_api_interfaces::Node::is_instance(scope, other_object)
    {
        let order_bit = disconnected_order_bit(args.this(), other_object);
        rv.set(
            v8::Integer::new_from_unsigned(
                scope,
                DOCUMENT_POSITION_DISCONNECTED
                    | DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC
                    | order_bit,
            )
            .into(),
        );
        return;
    }

    // Truly not a Node-like value — throw TypeError per WebIDL.
    let message = v8str(
        scope,
        "Failed to execute 'compareDocumentPosition' on 'Node': parameter 1 is not of type 'Node'.",
    );
    scope.throw_exception(v8::Exception::type_error(scope, message));
}

/// Stable PRECEDING/FOLLOWING choice for two disconnected nodes.
///
/// V8 identity hashes are i32 values, occasionally negative — promote to
/// `i64` to keep ordering well-defined. On the rare hash-collision case the
/// mirror property `(a, b) <-> (b, a)` would not hold; we accept that
/// trade-off since V8 hashes are 31 bits of entropy and the cross-tree
/// branch is itself a fallback. Spec only requires *some* consistent answer.
fn disconnected_order_bit(
    this: v8::Local<'_, v8::Object>,
    other: v8::Local<'_, v8::Object>,
) -> u32 {
    const DOCUMENT_POSITION_PRECEDING: u32 = 0x02;
    const DOCUMENT_POSITION_FOLLOWING: u32 = 0x04;
    let this_hash = this.get_identity_hash().get() as i64;
    let other_hash = other.get_identity_hash().get() as i64;
    if other_hash > this_hash {
        DOCUMENT_POSITION_FOLLOWING
    } else {
        DOCUMENT_POSITION_PRECEDING
    }
}

// DOM spec "locate a namespace": given a node and a prefix, return the
// associated namespace URI by walking ancestors and inspecting any xmlns
// attributes. The two prefix sentinels "xml" and "xmlns" map to fixed
// namespaces regardless of attributes.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

fn locate_namespace(
    runtime: &JsContextHost,
    start: DomHandle,
    prefix: Option<&str>,
) -> Option<String> {
    // Resolve the starting node into the first ELEMENT to inspect. Per spec:
    //   - Element node:           locate on the element itself
    //   - Document node:          delegate to documentElement (or null)
    //   - DocumentType/Fragment:  return null directly (no host concept here)
    //   - Anything else (Text /
    //     Comment / PI / Attr):   delegate to parentElement (or null)
    let mut element_handle = {
        let node = runtime.dom_host().node(start)?;
        if node.is_element() {
            start
        } else if node.is_document() {
            runtime
                .dom_host()
                .dom()
                .document_element_handle_for_document(start)?
        } else if node.is_document_fragment() {
            return None;
        } else {
            // Text / Comment / PI / Attribute / DocumentType: find first
            // ancestor element via parent walk.
            let mut current = start;
            loop {
                let parent = runtime.dom_host().parent_node(current)?;
                let parent_node = runtime.dom_host().node(parent)?;
                if parent_node.is_element() {
                    break parent;
                }
                if parent_node.is_document() || parent_node.is_document_fragment() {
                    return None;
                }
                current = parent;
            }
        }
    };

    // The fixed xml/xmlns namespace bindings only apply once the node can
    // resolve through an element. DocumentFragment, DocumentType, and
    // document-without-documentElement return null before reaching here.
    if let Some(p) = prefix {
        if p == "xml" {
            return Some(XML_NS.to_owned());
        }
        if p == "xmlns" {
            return Some(XMLNS_NS.to_owned());
        }
    }

    // Walk element-only ancestors looking for a namespace declaration.
    loop {
        let node = runtime.dom_host().node(element_handle)?;
        let element = node.as_element()?;
        let element_prefix = element.prefix().filter(|p| !p.is_empty());
        let element_namespace = element.namespace();
        if !element_namespace.is_empty() && element_prefix == prefix {
            return Some(element_namespace.to_owned());
        }
        for attr in element.attributes() {
            if attr.namespace() != XMLNS_NS {
                continue;
            }
            let attr_prefix = attr.prefix().filter(|p| !p.is_empty());
            match prefix {
                Some(p) => {
                    if attr_prefix == Some("xmlns") && attr.local_name() == p {
                        return non_empty_value(attr.value());
                    }
                }
                None => {
                    if attr_prefix.is_none() && attr.local_name() == "xmlns" {
                        return non_empty_value(attr.value());
                    }
                }
            }
        }
        // Walk up: only ELEMENT ancestors continue the search. Document,
        // DocumentFragment etc. terminate the walk per spec.
        let parent = runtime.dom_host().parent_node(element_handle)?;
        let parent_node = runtime.dom_host().node(parent)?;
        if !parent_node.is_element() {
            return None;
        }
        element_handle = parent;
    }
}

fn non_empty_value(value: &str) -> Option<String> {
    // Per spec, an xmlns attribute with empty value declares no default
    // namespace -> null. Non-xmlns empty values still propagate as Some("")
    // but we treat the spec-expected "null on empty" semantics here.
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

pub(in crate::native_bridge) fn node_lookup_namespace_uri_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "lookupNamespaceURI");
        rv.set_null();
        return;
    };
    let raw_prefix = args.get(0);
    let prefix = if raw_prefix.is_null_or_undefined() {
        None
    } else {
        match raw_prefix.to_string(scope) {
            Some(s) => {
                let raw = s.to_rust_string_lossy(scope);
                if raw.is_empty() { None } else { Some(raw) }
            }
            None => None,
        }
    };
    let runtime = unsafe { &*runtime_ptr };
    match locate_namespace(runtime, handle, prefix.as_deref()) {
        Some(ns) => match crate::util::v8_string(scope, &ns) {
            Some(s) => rv.set(s.into()),
            None => rv.set_null(),
        },
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_is_default_namespace_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "isDefaultNamespace");
        rv.set_bool(false);
        return;
    };
    let raw_namespace = args.get(0);
    let namespace = if raw_namespace.is_null_or_undefined() {
        None
    } else {
        match raw_namespace.to_string(scope) {
            Some(s) => {
                let raw = s.to_rust_string_lossy(scope);
                if raw.is_empty() { None } else { Some(raw) }
            }
            None => None,
        }
    };
    let runtime = unsafe { &*runtime_ptr };
    let default_namespace = locate_namespace(runtime, handle, None);
    rv.set_bool(default_namespace.as_deref() == namespace.as_deref());
}

pub(in crate::native_bridge) fn node_lookup_prefix_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "lookupPrefix");
        rv.set_null();
        return;
    };
    let raw = args.get(0);
    let namespace = if raw.is_null_or_undefined() {
        None
    } else {
        match raw.to_string(scope) {
            Some(s) => {
                let raw = s.to_rust_string_lossy(scope);
                if raw.is_empty() { None } else { Some(raw) }
            }
            None => None,
        }
    };
    let Some(namespace) = namespace else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    match locate_prefix(runtime, handle, &namespace) {
        Some(prefix) => match crate::util::v8_string(scope, &prefix) {
            Some(s) => rv.set(s.into()),
            None => rv.set_null(),
        },
        None => rv.set_null(),
    }
}

fn locate_prefix(runtime: &JsContextHost, start: DomHandle, namespace: &str) -> Option<String> {
    // DOM spec "locate a namespace prefix": walk up looking for an element
    // whose namespace matches and has a prefix; or an xmlns:p attribute whose
    // value matches. Same element-only-walk semantics as locate_namespace.
    let mut element_handle = {
        let node = runtime.dom_host().node(start)?;
        if node.is_element() {
            start
        } else if node.is_document() {
            runtime
                .dom_host()
                .dom()
                .document_element_handle_for_document(start)?
        } else {
            return None;
        }
    };

    loop {
        let node = runtime.dom_host().node(element_handle)?;
        let element = node.as_element()?;
        if element.namespace() == namespace
            && let Some(prefix) = element.prefix()
            && !prefix.is_empty()
        {
            return Some(prefix.to_owned());
        }
        for attr in element.attributes() {
            if attr.namespace() == XMLNS_NS
                && attr.prefix() == Some("xmlns")
                && attr.value() == namespace
            {
                return Some(attr.local_name().to_owned());
            }
        }
        let parent = runtime.dom_host().parent_node(element_handle)?;
        let parent_node = runtime.dom_host().node(parent)?;
        if !parent_node.is_element() {
            return None;
        }
        element_handle = parent;
    }
}

pub(in crate::native_bridge) fn node_get_root_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Node", "getRootNode");
        rv.set_null();
        return;
    };
    let this = v8::Global::new(scope, args.this());
    let this = v8::Local::new(scope, this);
    let receiver_is_detached = crate::native_bridge::document::detached_native_handle_for_runtime(
        scope,
        runtime_ptr,
        this,
    )
    .is_some();
    let composed = if args.length() > 0 {
        let options = args.get(0);
        if !options.is_null_or_undefined() && options.is_object() {
            options
                .to_object(scope)
                .and_then(|options| options.get(scope, v8str(scope, "composed").into()))
                .is_some_and(|value| value.boolean_value(scope))
        } else {
            false
        }
    } else {
        false
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(mut root_handle) = runtime.dom_host().root_node_handle(handle) else {
        rv.set_null();
        return;
    };
    if composed {
        while runtime.dom_host().is_shadow_root(root_handle) {
            let Some(host) = runtime.dom_host().shadow_root_host(root_handle) else {
                break;
            };
            let Some(next_root) = runtime.dom_host().root_node_handle(host) else {
                break;
            };
            root_handle = next_root;
        }
    }
    if receiver_is_detached {
        match crate::native_bridge::document::detached_native_object_for_handle(
            scope,
            runtime_ptr,
            root_handle,
        ) {
            Some(root) => rv.set(root.into()),
            None => rv.set_null(),
        }
        return;
    }
    match runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, root_handle)
    {
        Some(root) => rv.set(root.into()),
        None => rv.set_null(),
    }
}
