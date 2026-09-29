use super::*;

pub(in crate::native_bridge) fn node_contains_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NullableNodeArgs>(scope, &args) else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "contains") else {
        return;
    };
    let other = parsed
        .node
        .and_then(|other| NativeNodeReference::from_object(scope, other.0));
    rv.set_bool(other.is_some_and(|other| node.contains(scope, &other)));
}

pub(in crate::native_bridge) fn node_has_child_nodes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "hasChildNodes") else {
        return;
    };
    let has_children = node.tree_node().is_some_and(|node| {
        unsafe { &*node.runtime_ptr }
            .dom_host()
            .node(node.handle)
            .is_some_and(Node::has_child_nodes)
    });
    rv.set_bool(has_children);
}

pub(in crate::native_bridge) fn node_is_same_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NullableNodeArgs>(scope, &args) else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "isSameNode") else {
        return;
    };
    let other = parsed
        .node
        .and_then(|other| NativeNodeReference::from_object(scope, other.0));
    rv.set_bool(other.is_some_and(|other| node.is_same(scope, &other)));
}

pub(in crate::native_bridge) fn node_is_equal_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NullableNodeArgs>(scope, &args) else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "isEqualNode") else {
        return;
    };
    let other = parsed
        .node
        .and_then(|other| NativeNodeReference::from_object(scope, other.0));
    rv.set_bool(other.is_some_and(|other| node.is_equal(scope, &other)));
}

pub(in crate::native_bridge) fn node_compare_document_position_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RequiredNodeArgs>(scope, &args) else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "compareDocumentPosition")
    else {
        return;
    };
    let Some(other) = NativeNodeReference::from_object(scope, parsed.node.0) else {
        throw_type_error(scope, "Node data is unavailable");
        return;
    };
    let position = node.compare_position(scope, &other);
    rv.set(v8::Integer::new_from_unsigned(scope, position.into()).into());
}

// DOM spec "locate a namespace": given a node and a prefix, return the
// associated namespace URI by walking ancestors and inspecting any xmlns
// attributes. The two prefix sentinels "xml" and "xmlns" map to fixed
// namespaces regardless of attributes.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

// Attr is resolved to its owner by NativeNodeReference before reaching here.
// Both namespace algorithms use the same interface-specific starting element.
fn namespace_lookup_element(runtime: &JsContextHost, start: DomHandle) -> Option<DomHandle> {
    let node = runtime.dom_host().node(start)?;
    match node.node_type() {
        NodeType::Element => Some(start),
        NodeType::Document => runtime
            .dom_host()
            .dom()
            .document_element_handle_for_document(start),
        NodeType::DocumentType | NodeType::DocumentFragment => None,
        _ => {
            let parent = node.parent_node()?;
            runtime.dom_host().node(parent)?.as_element()?;
            Some(parent)
        }
    }
}

fn locate_namespace(
    runtime: &JsContextHost,
    start: DomHandle,
    prefix: Option<&str>,
) -> Option<String> {
    let mut element_handle = namespace_lookup_element(runtime, start)?;

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

pub(in crate::native_bridge) fn node_lookup_namespace_uri_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NodeNamespaceArgs>(scope, &args) else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "lookupNamespaceURI") else {
        return;
    };
    let prefix = parsed.namespace.filter(|value| !value.is_empty());
    let namespace = node.tree_or_owner(scope).and_then(|node| {
        locate_namespace(
            unsafe { &*node.runtime_ptr },
            node.handle,
            prefix.as_deref(),
        )
    });
    match namespace.and_then(|value| v8_string(scope, &value)) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn node_is_default_namespace_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NodeNamespaceArgs>(scope, &args) else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "isDefaultNamespace") else {
        return;
    };
    let namespace = parsed.namespace.filter(|value| !value.is_empty());
    let default_namespace = node
        .tree_or_owner(scope)
        .and_then(|node| locate_namespace(unsafe { &*node.runtime_ptr }, node.handle, None));
    rv.set_bool(default_namespace == namespace);
}

pub(in crate::native_bridge) fn node_lookup_prefix_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<NodeNamespaceArgs>(scope, &args) else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "lookupPrefix") else {
        return;
    };
    let prefix = parsed
        .namespace
        .filter(|value| !value.is_empty())
        .and_then(|namespace| {
            node.tree_or_owner(scope).and_then(|node| {
                locate_prefix(unsafe { &*node.runtime_ptr }, node.handle, &namespace)
            })
        });
    match prefix.and_then(|value| v8_string(scope, &value)) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn locate_prefix(runtime: &JsContextHost, start: DomHandle, namespace: &str) -> Option<String> {
    let mut element_handle = namespace_lookup_element(runtime, start)?;

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

pub(in crate::native_bridge) fn node_get_root_node_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "getRootNode") else {
        return;
    };
    let options =
        webidl::dictionary_arg(&args, 0, webidl::Context::argument("Node.getRootNode", 1))
            .and_then(|object| {
                object
                    .map(|object| {
                        webidl::parse_dictionary_object::<GetRootNodeOptions>(scope, object)
                    })
                    .transpose()
            });
    let composed = match options {
        Ok(options) => options.unwrap_or_default().composed,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Some(node) = node.tree_node() else {
        rv.set(args.this().into());
        return;
    };
    let (runtime_ptr, handle) = (node.runtime_ptr, node.handle);
    let receiver_is_detached = crate::native_bridge::document::detached_native_handle_for_runtime(
        scope,
        runtime_ptr,
        args.this(),
    )
    .is_some();
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
    set_wrapped_handle_or_null_for_receiver(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        Some(root_handle),
    );
}
