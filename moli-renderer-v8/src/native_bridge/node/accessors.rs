use super::*;
use crate::dom::native::DomStringValue;
use crate::util::v8_string_from_utf16_units;

pub(super) fn node_node_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "nodeType") else {
        return;
    };
    let kind = match node {
        NativeNodeReference::Attr(_) => 2,
        NativeNodeReference::Tree { node, .. } => {
            let Some(node) = unsafe { &*node.runtime_ptr }.dom_host().node(node.handle) else {
                return;
            };
            node.node_type() as i32
        }
    };
    rv.set_int32(kind);
}

pub(super) fn node_node_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "nodeName") else {
        return;
    };
    let name = match node {
        NativeNodeReference::Attr(attr) => attr.name(scope).map(DomStringValue::from),
        NativeNodeReference::Tree { node, .. } => {
            let runtime = unsafe { &*node.runtime_ptr };
            runtime.dom_host().node(node.handle).map(|value| {
                element_name_for_owner_document(runtime, node.handle)
                    .map(DomStringValue::from)
                    .unwrap_or_else(|| value.node_name_value())
            })
        }
    };
    if let Some(name) =
        name.and_then(|name| crate::util::v8_string_from_dom_string_value(scope, &name))
    {
        rv.set(name.into());
    }
}

pub(super) fn node_node_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "nodeValue") else {
        return;
    };
    let value = match node {
        NativeNodeReference::Attr(attr) => Some(attr.value(scope)),
        NativeNodeReference::Tree { node, .. } => unsafe { &*node.runtime_ptr }
            .dom_host()
            .node(node.handle)
            .and_then(Node::character_data_value)
            .cloned(),
    };
    match value.and_then(|value| v8_string_from_utf16_units(scope, &value.utf16_units())) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn nullable_node_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
) -> Option<DomStringValue> {
    if value.is_null_or_undefined() {
        return Some(DomStringValue::default());
    }
    match webidl::convert::<webidl::DomString16>(
        scope,
        value,
        webidl::Context::member("Node", member),
    ) {
        Ok(value) => Some(DomStringValue::from_utf16(&value.0)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(super) fn node_node_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    // Receiver validation is generated. Conversion precedes native resolution
    // because toString can adopt the node into another document.
    let Some(value) = nullable_node_string(scope, args.get(0), "nodeValue") else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "nodeValue") else {
        return;
    };
    match node {
        NativeNodeReference::Attr(attr) => attr.set_value(scope, value),
        NativeNodeReference::Tree { node, .. } => {
            let runtime = unsafe { &mut *node.runtime_ptr };
            let Some(removed_count) = runtime
                .character_data_utf16_units(node.handle)
                .map(|value| value.len() as u32)
            else {
                return;
            };
            let units = value.utf16_units();
            let inserted_count = units.len() as u32;
            runtime.set_character_data_utf16_units_for_edit(
                scope,
                node.runtime_ptr,
                node.handle,
                &units,
            );
            context_bootstrap::live_ranges_character_data_reset(
                scope,
                node.handle,
                removed_count,
                inserted_count,
            );
        }
    }
}

pub(super) fn node_is_connected_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "isConnected") else {
        return;
    };
    rv.set_bool(node.tree_node().is_some_and(|node| {
        unsafe { &*node.runtime_ptr }.node_is_connected_for_web_api(node.handle)
    }));
}

pub(super) fn node_owner_document_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "ownerDocument") else {
        return;
    };
    match node {
        NativeNodeReference::Attr(attr) => match attr.owner_document(scope) {
            Some(document) => rv.set(document.into()),
            None => rv.set_null(),
        },
        NativeNodeReference::Tree { node, .. } => {
            let owner = unsafe { &*node.runtime_ptr }
                .dom_host()
                .node(node.handle)
                .and_then(Node::owner_document);
            set_wrapped_handle_or_null_for_receiver(
                scope,
                &mut rv,
                node.runtime_ptr,
                args.this(),
                owner,
            );
        }
    }
}

pub(super) fn node_base_uri_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "baseURI") else {
        return;
    };
    let node = match node {
        NativeNodeReference::Tree { node, .. } => Some(node),
        NativeNodeReference::Attr(attr) => attr
            .owner_document(scope)
            .and_then(|document| reference::TreeNodeReference::from_object(scope, document)),
    };
    let Some(node) = node else {
        return;
    };
    let runtime = unsafe { &*node.runtime_ptr };
    let Some(document) = runtime.dom_host().owner_document_handle(node.handle) else {
        return;
    };
    let url = runtime.document_base_url_for_handle(document);
    if let Some(value) = v8_string(scope, url.as_str()) {
        rv.set(value.into());
    }
}

enum NodeTreeRelation {
    Parent,
    ParentElement,
    FirstChild,
    LastChild,
    PreviousSibling,
    NextSibling,
}

fn node_tree_relation_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    relation: NodeTreeRelation,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "tree relation") else {
        return;
    };
    let Some(node) = node.tree_node() else {
        rv.set_null();
        return;
    };
    let dom = unsafe { &*node.runtime_ptr }.dom_host();
    let related = dom.node(node.handle).and_then(|node| match relation {
        NodeTreeRelation::Parent => node.parent_node(),
        NodeTreeRelation::ParentElement => node
            .parent_node()
            .filter(|parent| dom.node(*parent).is_some_and(Node::is_element)),
        NodeTreeRelation::FirstChild => node.first_child(),
        NodeTreeRelation::LastChild => node.last_child(),
        NodeTreeRelation::PreviousSibling => node.prev_sibling(),
        NodeTreeRelation::NextSibling => node.next_sibling(),
    });
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, node.runtime_ptr, args.this(), related);
}

pub(super) fn node_parent_node_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    node_tree_relation_getter(scope, args, rv, NodeTreeRelation::Parent);
}
pub(super) fn node_parent_element_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    node_tree_relation_getter(scope, args, rv, NodeTreeRelation::ParentElement);
}
pub(super) fn node_first_child_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    node_tree_relation_getter(scope, args, rv, NodeTreeRelation::FirstChild);
}
pub(super) fn node_last_child_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    node_tree_relation_getter(scope, args, rv, NodeTreeRelation::LastChild);
}
pub(super) fn node_previous_sibling_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    node_tree_relation_getter(scope, args, rv, NodeTreeRelation::PreviousSibling);
}
pub(super) fn node_next_sibling_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    node_tree_relation_getter(scope, args, rv, NodeTreeRelation::NextSibling);
}

pub(super) fn node_child_nodes_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "childNodes") else {
        return;
    };
    match node {
        NativeNodeReference::Attr(attr) => {
            if let Some(list) = attr.child_nodes(scope) {
                rv.set(list.into());
            }
        }
        NativeNodeReference::Tree { object, node } => {
            let Some(context) = object.get_creation_context(scope) else {
                return;
            };
            let scope = &mut v8::ContextScope::new(scope, context);
            let collection = super::super::collections::build_live_collection_for_node(
                scope,
                node.runtime_ptr,
                node.handle,
                CollectionKind::NodeList,
                LiveCollectionQueryKind::ChildNodes,
                None,
                false,
            );
            rv.set(collection.into());
        }
    }
}

pub(in crate::native_bridge) fn node_text_content_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "textContent") else {
        return;
    };
    let units = match node {
        NativeNodeReference::Attr(attr) => Some(attr.value(scope).utf16_units().into_owned()),
        NativeNodeReference::Tree { node, .. } => {
            let runtime = unsafe { &*node.runtime_ptr };
            runtime
                .dom_host()
                .node(node.handle)
                .filter(|node| !node.is_document() && node.as_document_type().is_none())
                .map(|node| node.text_content_utf16_units(runtime.dom_host().dom()))
        }
    };
    match units.and_then(|units| v8_string_from_utf16_units(scope, &units)) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

pub(super) fn node_text_content_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(value) = nullable_node_string(scope, args.get(0), "textContent") else {
        return;
    };
    let Some(node) = NativeNodeReference::receiver(scope, args.this(), "textContent") else {
        return;
    };
    match node {
        NativeNodeReference::Attr(attr) => attr.set_value(scope, value),
        NativeNodeReference::Tree { node, .. } => {
            let runtime = unsafe { &*node.runtime_ptr };
            let Some(native) = runtime.dom_host().node(node.handle) else {
                return;
            };
            if native.is_document() || native.as_document_type().is_some() {
                return;
            }
            let removed_count = runtime
                .character_data_utf16_units(node.handle)
                .map(|units| units.len() as u32);
            let inserted_count = value.utf16_units().len() as u32;
            set_text_content_in_reaction_scope(scope, node.runtime_ptr, node.handle, value);
            if let Some(removed_count) = removed_count {
                context_bootstrap::live_ranges_character_data_reset(
                    scope,
                    node.handle,
                    removed_count,
                    inserted_count,
                );
            }
        }
    }
}
