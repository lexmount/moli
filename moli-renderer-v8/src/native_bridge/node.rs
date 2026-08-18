use crate::{
    context_bootstrap,
    dom::native::{DocumentType, Node, NodeType},
    webidl,
};
use moli_webapi_declare::WebApiFunctionTemplate;

use super::super::{
    document_runtime::DomHandle,
    util::{
        call_global_bridge_method, context_host_ptr_from_global_bridge, get_private_value,
        throw_type_error, v8_string, v8str,
    },
};
use super::{
    CollectionKind, JsContextHost, LiveCollectionQueryKind, callback_arg_string,
    runtime_ptr_from_object, set_wrapped_handle_or_null, throw_dom_exception,
};

mod bridge_callbacks;
mod character_data;
mod errors;
mod foreign;
mod metadata;
mod mutation;
mod tree;

pub(super) use self::bridge_callbacks::*;
pub(crate) use self::character_data::install_character_data_template_bindings;
pub(super) use self::character_data::*;
pub(super) use self::errors::*;
pub(crate) use self::foreign::node_or_foreign_arg_handle_allow_detached;
pub(super) use self::foreign::*;
use self::metadata::*;
pub(crate) use self::mutation::validate_pre_insert_handles;
pub(super) use self::mutation::*;
pub(super) use self::tree::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Node", enumerable)]
struct NodePrototypeReflectionDeclaration {
    #[webapi(accessor_property = "nodeType", getter = node_node_type_getter_function)]
    node_type: (),
    #[webapi(accessor_property = "nodeName", getter = node_node_name_getter_function)]
    node_name: (),
    #[webapi(
        accessor_property = "nodeValue",
        getter = node_node_value_getter_function,
        setter = node_node_value_setter_function
    )]
    node_value: (),
    #[webapi(accessor_property = "isConnected", getter = node_is_connected_getter_function)]
    is_connected: (),
    #[webapi(
        accessor_property = "ownerDocument",
        getter = node_owner_document_getter_function
    )]
    owner_document: (),
    #[webapi(accessor_property = "baseURI", getter = node_base_uri_getter_function)]
    base_uri: (),
    #[webapi(accessor_property, getter = node_parent_node_getter_function)]
    parent_node: (),
    #[webapi(accessor_property, getter = node_parent_element_getter_function)]
    parent_element: (),
    #[webapi(accessor_property, getter = node_child_nodes_getter_function)]
    child_nodes: (),
    #[webapi(accessor_property, getter = node_first_child_getter_function)]
    first_child: (),
    #[webapi(accessor_property, getter = node_last_child_getter_function)]
    last_child: (),
    #[webapi(accessor_property, getter = node_previous_sibling_getter_function)]
    previous_sibling: (),
    #[webapi(accessor_property, getter = node_next_sibling_getter_function)]
    next_sibling: (),
    #[webapi(
        accessor_property,
        getter = node_text_content_getter_function,
        setter = node_text_content_setter_function
    )]
    text_content: (),
    #[webapi(method, length = 1, callback = node_append_child_prototype_callback)]
    append_child: (),
    #[webapi(method, length = 2, callback = node_insert_before_prototype_callback)]
    insert_before: (),
    #[webapi(method, length = 1, callback = node_remove_child_prototype_callback)]
    remove_child: (),
    #[webapi(method, length = 2, callback = node_replace_child_prototype_callback)]
    replace_child: (),
    #[webapi(method, length = 0, callback = node_clone_node_callback)]
    clone_node: (),
    #[webapi(method, length = 1, callback = node_contains_callback)]
    contains: (),
    #[webapi(method, length = 0, callback = node_has_child_nodes_callback)]
    has_child_nodes: (),
    #[webapi(method, length = 1, callback = node_is_same_node_callback)]
    is_same_node: (),
    #[webapi(method, length = 1, callback = node_is_equal_node_callback)]
    is_equal_node: (),
    #[webapi(method, length = 1, callback = node_compare_document_position_callback)]
    compare_document_position: (),
    #[webapi(method, length = 0, callback = node_get_root_node_callback)]
    get_root_node: (),
    #[webapi(method, length = 1, callback = node_lookup_prefix_callback)]
    lookup_prefix: (),
    #[webapi(method = "lookupNamespaceURI", length = 1, callback = node_lookup_namespace_uri_callback)]
    lookup_namespace_uri: (),
    #[webapi(method, length = 1, callback = node_is_default_namespace_callback)]
    is_default_namespace: (),
    #[webapi(method, length = 0, callback = node_normalize_callback)]
    normalize: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ParentNode", enumerable)]
struct ParentNodePrototypeDeclaration {
    #[webapi(accessor_property, getter = parent_node_children_getter_function)]
    children: (),
    #[webapi(accessor_property, getter = parent_node_first_element_child_getter_function)]
    first_element_child: (),
    #[webapi(accessor_property, getter = parent_node_last_element_child_getter_function)]
    last_element_child: (),
    #[webapi(accessor_property, getter = parent_node_child_element_count_getter_function)]
    child_element_count: (),
    #[webapi(method, length = 0, callback = node_prepend_callback)]
    prepend: (),
    #[webapi(method, length = 0, callback = node_append_callback)]
    append: (),
    #[webapi(method, length = 0, callback = node_replace_children_callback)]
    replace_children: (),
    #[webapi(method = "moveBefore", length = 2, callback = node_move_before_callback)]
    move_before: (),
    #[webapi(method, length = 1, callback = super::element::node_query_selector_callback)]
    query_selector: (),
    #[webapi(method, length = 1, callback = super::element::node_query_selector_all_callback)]
    query_selector_all: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ChildNode", enumerable)]
struct ChildNodePrototypeDeclaration {
    #[webapi(method, length = 0, callback = node_before_callback)]
    before: (),
    #[webapi(method, length = 0, callback = node_after_callback)]
    after: (),
    #[webapi(method, length = 0, callback = node_replace_with_callback)]
    replace_with: (),
    #[webapi(method, length = 0, callback = node_remove_callback)]
    remove: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "NonDocumentTypeChildNode", enumerable)]
struct NonDocumentTypeChildNodePrototypeDeclaration {
    #[webapi(accessor_property, getter = non_document_type_child_node_previous_element_sibling_getter_function)]
    previous_element_sibling: (),
    #[webapi(accessor_property, getter = non_document_type_child_node_next_element_sibling_getter_function)]
    next_element_sibling: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DocumentType", enumerable)]
struct DocumentTypePrototypeDeclaration {
    #[webapi(accessor_property, getter = document_type_name_getter_function)]
    name: (),
    #[webapi(accessor_property = "publicId", getter = document_type_public_id_getter_function)]
    public_id: (),
    #[webapi(accessor_property = "systemId", getter = document_type_system_id_getter_function)]
    system_id: (),
}

fn set_document_type_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<String>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(value) = value else {
        rv.set_undefined();
        return;
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn detached_bridge_value_for_this<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    method: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, receiver, super::document::DETACHED_STATE_SLOT)?;
    call_global_bridge_method(scope, method, &[receiver.into()])
}

pub(super) fn receiver_has_detached_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, receiver, super::document::DETACHED_STATE_SLOT).is_some()
}

pub(super) fn throw_incompatible_getter_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    interface: &str,
    member: &str,
) {
    webidl::throw_type_error(
        scope,
        &format!("{interface}.{member} getter called on incompatible receiver."),
    );
}

pub(super) fn throw_incompatible_setter_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    interface: &str,
    member: &str,
) {
    webidl::throw_type_error(
        scope,
        &format!("{interface}.{member} setter called on incompatible receiver."),
    );
}

pub(super) fn throw_incompatible_method_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    interface: &str,
    method: &str,
) {
    webidl::throw_type_error(
        scope,
        &format!("Failed to execute '{method}' on '{interface}': Illegal invocation."),
    );
}

fn node_type_for_handle(runtime: &JsContextHost, handle: DomHandle) -> Option<NodeType> {
    runtime.dom_host().node(handle).map(Node::node_type)
}

fn is_parent_node_receiver(runtime: &JsContextHost, handle: DomHandle) -> bool {
    node_type_for_handle(runtime, handle).is_some_and(|node_type| {
        matches!(
            node_type,
            NodeType::Document | NodeType::DocumentFragment | NodeType::Element
        )
    })
}

fn is_child_node_receiver(runtime: &JsContextHost, handle: DomHandle) -> bool {
    node_type_for_handle(runtime, handle).is_some_and(|node_type| {
        matches!(
            node_type,
            NodeType::DocumentType
                | NodeType::Element
                | NodeType::Text
                | NodeType::CDataSection
                | NodeType::Comment
                | NodeType::ProcessingInstruction
        )
    })
}

fn is_non_document_type_child_node_receiver(runtime: &JsContextHost, handle: DomHandle) -> bool {
    node_type_for_handle(runtime, handle).is_some_and(|node_type| {
        matches!(
            node_type,
            NodeType::Element
                | NodeType::Text
                | NodeType::CDataSection
                | NodeType::Comment
                | NodeType::ProcessingInstruction
        )
    })
}

pub(super) fn require_parent_node_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
    member: &str,
    is_method: bool,
) -> bool {
    if is_parent_node_receiver(runtime, handle) {
        return true;
    }
    if is_method {
        throw_incompatible_method_receiver(scope, "ParentNode", member);
    } else {
        throw_incompatible_getter_receiver(scope, "ParentNode", member);
    }
    false
}

pub(super) fn require_child_node_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
    member: &str,
) -> bool {
    if is_child_node_receiver(runtime, handle) {
        return true;
    }
    throw_incompatible_method_receiver(scope, "ChildNode", member);
    false
}

pub(super) fn require_non_document_type_child_node_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
    member: &str,
) -> bool {
    if is_non_document_type_child_node_receiver(runtime, handle) {
        return true;
    }
    throw_incompatible_getter_receiver(scope, "NonDocumentTypeChildNode", member);
    false
}

pub(super) fn require_element_getter_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
    member: &str,
) -> bool {
    if node_is_element(runtime, handle) {
        return true;
    }
    throw_incompatible_getter_receiver(scope, "Element", member);
    false
}

pub(super) fn require_element_setter_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
    member: &str,
) -> bool {
    if node_is_element(runtime, handle) {
        return true;
    }
    throw_incompatible_setter_receiver(scope, "Element", member);
    false
}

pub(super) fn require_element_method_receiver(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
    method: &str,
) -> bool {
    if node_is_element(runtime, handle) {
        return true;
    }
    throw_incompatible_method_receiver(scope, "Element", method);
    false
}

fn live_document_type_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    member: &str,
    value: for<'a> fn(&'a DocumentType) -> &'a str,
) -> std::result::Result<String, ()> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, args) else {
        webidl::throw_type_error(
            scope,
            &format!("DocumentType.{member} getter called on incompatible receiver."),
        );
        return Err(());
    };
    let Some(value) = (unsafe { &*runtime_ptr })
        .dom_host()
        .node(handle)
        .and_then(Node::as_document_type)
        .map(value)
        .map(str::to_owned)
    else {
        webidl::throw_type_error(
            scope,
            &format!("DocumentType.{member} getter called on incompatible receiver."),
        );
        return Err(());
    };
    Ok(value)
}

fn document_type_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = super::document::detached_doctype_name(scope, args.this())
        .or_else(|| live_document_type_value(scope, &args, "name", DocumentType::name).ok());
    set_document_type_string_value(scope, value, rv);
}

fn document_type_public_id_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = super::document::detached_doctype_public_id(scope, args.this()).or_else(|| {
        live_document_type_value(scope, &args, "publicId", DocumentType::public_id).ok()
    });
    set_document_type_string_value(scope, value, rv);
}

fn document_type_system_id_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = super::document::detached_doctype_system_id(scope, args.this()).or_else(|| {
        live_document_type_value(scope, &args, "systemId", DocumentType::system_id).ok()
    });
    set_document_type_string_value(scope, value, rv);
}

fn node_node_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedNodeType")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "nodeType");
        rv.set_null();
        return;
    };
    let Some(node) = unsafe { &*runtime_ptr }.dom_host().node(handle) else {
        rv.set_null();
        return;
    };
    rv.set(v8::Integer::new(scope, i32::from(node.node_type() as u8)).into());
}

fn node_node_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedNodeName")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "nodeName");
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(node) = runtime.dom_host().node(handle) else {
        rv.set_null();
        return;
    };
    let name = element_name_for_owner_document(runtime, handle).unwrap_or_else(|| node.node_name());
    let Some(value) = v8_string(scope, &name) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn node_node_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedNodeValue")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "nodeValue");
        rv.set_null();
        return;
    };
    let Some(node) = unsafe { &*runtime_ptr }.dom_host().node(handle) else {
        rv.set_null();
        return;
    };
    match node.node_value() {
        Some(value) => {
            let value = v8_string(scope, value).unwrap_or_else(|| v8::String::empty(scope));
            rv.set(value.into());
        }
        None => rv.set_null(),
    }
}

fn node_node_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let raw_value = args.get(0);
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        if !receiver_has_detached_state(scope, args.this()) {
            throw_incompatible_setter_receiver(scope, "Node", "nodeValue");
            return;
        }
        let node = args.this();
        let _ =
            call_global_bridge_method(scope, "__setDetachedNodeValue", &[node.into(), raw_value]);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(node) = runtime.dom_host().node(handle) else {
        return;
    };
    if node.node_value().is_none() {
        return;
    }
    let value = if raw_value.is_null_or_undefined() {
        String::new()
    } else {
        match webidl::convert::<webidl::DomString>(
            scope,
            raw_value,
            webidl::Context::member("Node", "nodeValue"),
        ) {
            Ok(value) => value.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return;
            }
        }
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let removed_count = runtime
        .character_data_utf16_units(handle)
        .map(|units| units.len() as u32);
    let inserted_count = value.encode_utf16().count() as u32;
    let _ = runtime.set_text_content(scope, runtime_ptr, handle, &value);
    if let Some(removed_count) = removed_count {
        context_bootstrap::live_ranges_character_data_reset(
            scope,
            handle,
            removed_count,
            inserted_count,
        );
    }
}

fn node_is_connected_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedIsConnected")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "isConnected");
        rv.set_bool(false);
        return;
    };
    let connected = unsafe { &*runtime_ptr }.node_is_connected_for_web_api(handle);
    rv.set(v8::Boolean::new(scope, connected).into());
}

fn node_owner_document_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedOwnerDocument")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "ownerDocument");
        rv.set_null();
        return;
    };
    let owner = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::owner_document);
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, owner);
}

fn node_base_uri_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_getter_receiver(scope, "Node", "baseURI");
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let document_handle = if node_is_document(runtime, handle) {
        Some(handle)
    } else {
        runtime
            .dom_host()
            .node(handle)
            .and_then(Node::owner_document)
    };
    let Some(document_handle) = document_handle else {
        rv.set_undefined();
        return;
    };
    let url = if document_handle == runtime.dom_host().document_handle() {
        runtime
            .dom_host()
            .document_base_url()
            .unwrap_or_else(|| runtime.host_document().url().clone())
    } else if let Some(child_handle) =
        runtime.child_browsing_context_host_for_document_handle(document_handle)
        && let Some(base_url) = runtime.child_browsing_context_base_url(child_handle)
    {
        base_url
    } else {
        runtime
            .dom_host()
            .node(document_handle)
            .and_then(Node::as_document)
            .map(|document| document.base_url().clone())
            .unwrap_or_else(|| runtime.host_document().url().clone())
    };
    let Some(value) = v8_string(scope, url.as_str()) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn node_parent_node_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedParentNode")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "parentNode");
        rv.set_null();
        return;
    };
    let parent = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node);
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, parent);
}

fn node_parent_element_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedParentElement")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "parentElement");
        rv.set_null();
        return;
    };
    let parent = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)
        .filter(|parent| node_is_element(unsafe { &*runtime_ptr }, *parent));
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, parent);
}

fn node_child_nodes_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedChildNodes")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "childNodes");
        rv.set_null();
        return;
    };
    let collection = super::collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::NodeList,
        LiveCollectionQueryKind::ChildNodes,
        None,
        false,
    );
    rv.set(collection.into());
}

fn node_first_child_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedFirstChild")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "firstChild");
        rv.set_null();
        return;
    };
    let child = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::first_child);
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, child);
}

fn node_last_child_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedLastChild")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "lastChild");
        rv.set_null();
        return;
    };
    let child = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::last_child);
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, child);
}

fn node_previous_sibling_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedPreviousSibling")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "previousSibling");
        rv.set_null();
        return;
    };
    let sibling = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::prev_sibling);
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, sibling);
}

fn node_next_sibling_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedNextSibling")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "nextSibling");
        rv.set_null();
        return;
    };
    let sibling = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::next_sibling);
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, sibling);
}

pub(in crate::native_bridge) fn node_text_content_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedTextContent")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "Node", "textContent");
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(node) = runtime.dom_host().node(handle) else {
        rv.set_null();
        return;
    };
    if node.is_document() || node.as_document_type().is_some() {
        rv.set_null();
        return;
    }
    let Some(value) = runtime
        .dom_host()
        .node(handle)
        .map(|node| node.text_content(runtime.dom_host().dom()))
    else {
        rv.set_null();
        return;
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn node_text_content_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let raw_value = args.get(0);
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        if !receiver_has_detached_state(scope, args.this()) {
            throw_incompatible_setter_receiver(scope, "Node", "textContent");
            return;
        }
        super::document::set_detached_node_text_content(scope, args.this(), raw_value);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(node) = runtime.dom_host().node(handle) else {
        return;
    };
    if node.is_document() || node.as_document_type().is_some() {
        return;
    }
    let value = if raw_value.is_null_or_undefined() {
        String::new()
    } else {
        let Some(value) = raw_value.to_string(scope) else {
            return;
        };
        value.to_rust_string_lossy(scope)
    };
    let removed_count = runtime
        .character_data_utf16_units(handle)
        .map(|units| units.len() as u32);
    let inserted_count = value.encode_utf16().count() as u32;
    let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &value);
    if let Some(removed_count) = removed_count {
        context_bootstrap::live_ranges_character_data_reset(
            scope,
            handle,
            removed_count,
            inserted_count,
        );
    }
}

pub(super) fn element_name_for_owner_document(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let dom_host = runtime.dom_host();
    let node = dom_host.node(handle)?;
    let element = node.as_element()?;
    let owner_document = node.owner_document()?;
    let owner_is_html = dom_host
        .node(owner_document)
        .and_then(Node::as_document)
        .is_some_and(|document| document.is_html_document());
    (!owner_is_html).then(|| element.qualified_name())
}

fn parent_node_children_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedChildren")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "ParentNode", "children");
        rv.set_null();
        return;
    };
    if !require_parent_node_receiver(scope, unsafe { &*runtime_ptr }, handle, "children", false) {
        rv.set_null();
        return;
    }
    let collection = super::collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::HtmlCollection,
        LiveCollectionQueryKind::Children,
        None,
        false,
    );
    rv.set(collection.into());
}

fn parent_node_first_element_child_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedFirstElementChild")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "ParentNode", "firstElementChild");
        rv.set_null();
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "firstElementChild",
        false,
    ) {
        rv.set_null();
        return;
    }
    let child = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.first_element_child(unsafe { &*runtime_ptr }.dom_host().dom()));
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, child);
}

fn parent_node_last_element_child_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedLastElementChild")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "ParentNode", "lastElementChild");
        rv.set_null();
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "lastElementChild",
        false,
    ) {
        rv.set_null();
        return;
    }
    let child = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.last_element_child(unsafe { &*runtime_ptr }.dom_host().dom()));
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, child);
}

fn parent_node_child_element_count_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedChildElementCount")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "ParentNode", "childElementCount");
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "childElementCount",
        false,
    ) {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    }
    let count = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .map(|node| node.child_element_count(unsafe { &*runtime_ptr }.dom_host().dom()))
        .unwrap_or(0);
    rv.set(v8::Integer::new(scope, count as i32).into());
}

fn non_document_type_child_node_previous_element_sibling_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedPreviousElementSibling")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(
            scope,
            "NonDocumentTypeChildNode",
            "previousElementSibling",
        );
        rv.set_null();
        return;
    };
    if !require_non_document_type_child_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "previousElementSibling",
    ) {
        rv.set_null();
        return;
    }
    let sibling = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.previous_element_sibling(unsafe { &*runtime_ptr }.dom_host().dom()));
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, sibling);
}

fn non_document_type_child_node_next_element_sibling_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if let Some(value) =
            detached_bridge_value_for_this(scope, args.this(), "__detachedNextElementSibling")
        {
            rv.set(value);
            return;
        }
        throw_incompatible_getter_receiver(scope, "NonDocumentTypeChildNode", "nextElementSibling");
        rv.set_null();
        return;
    };
    if !require_non_document_type_child_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "nextElementSibling",
    ) {
        rv.set_null();
        return;
    }
    let sibling = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.next_element_sibling(unsafe { &*runtime_ptr }.dom_host().dom()));
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, sibling);
}

fn node_append_child_prototype_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if node_runtime_and_handle_from_args(scope, &args).is_ok() {
        node_append_child_callback(scope, args, rv);
    } else if receiver_has_detached_state(scope, args.this()) {
        super::document::detached_append_child_method_callback(scope, args, rv);
    } else {
        throw_incompatible_method_receiver(scope, "Node", "appendChild");
    }
}

fn node_insert_before_prototype_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if node_runtime_and_handle_from_args(scope, &args).is_ok() {
        node_insert_before_callback(scope, args, rv);
    } else if receiver_has_detached_state(scope, args.this()) {
        super::document::detached_insert_before_method_callback(scope, args, rv);
    } else {
        throw_incompatible_method_receiver(scope, "Node", "insertBefore");
    }
}

fn node_remove_child_prototype_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if node_runtime_and_handle_from_args(scope, &args).is_ok() {
        node_remove_child_callback(scope, args, rv);
    } else if receiver_has_detached_state(scope, args.this()) {
        super::document::detached_remove_child_method_callback(scope, args, rv);
    } else {
        throw_incompatible_method_receiver(scope, "Node", "removeChild");
    }
}

fn node_replace_child_prototype_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if node_runtime_and_handle_from_args(scope, &args).is_ok() {
        node_replace_child_callback(scope, args, rv);
    } else if receiver_has_detached_state(scope, args.this()) {
        super::document::detached_replace_child_method_callback(scope, args, rv);
    } else {
        throw_incompatible_method_receiver(scope, "Node", "replaceChild");
    }
}

pub(crate) fn install_node_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    if interface_name == "Node" {
        NodePrototypeReflectionDeclaration::initialize_prototype_template(scope, prototype);
    }
    if matches!(interface_name, "Document" | "DocumentFragment" | "Element") {
        ParentNodePrototypeDeclaration::initialize_prototype_template(scope, prototype);
    }
    if matches!(interface_name, "DocumentType" | "Element" | "CharacterData") {
        ChildNodePrototypeDeclaration::initialize_prototype_template(scope, prototype);
    }
    if matches!(interface_name, "Element" | "CharacterData") {
        NonDocumentTypeChildNodePrototypeDeclaration::initialize_prototype_template(
            scope, prototype,
        );
    }
    if interface_name == "DocumentType" {
        DocumentTypePrototypeDeclaration::initialize_prototype_template(scope, prototype);
    }
}

pub(crate) fn node_runtime_and_handle_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> std::result::Result<(*mut JsContextHost, DomHandle), String> {
    let (runtime_ptr, handle) = super::bridge_handle_from_object(scope, object)?;
    match handle {
        super::BridgeHandle::Node(handle) => Ok((runtime_ptr, handle)),
        super::BridgeHandle::Window
        | super::BridgeHandle::ClassList(_, _)
        | super::BridgeHandle::Dataset(_)
        | super::BridgeHandle::Style(_)
        | super::BridgeHandle::ComputedStyle(_, _) => {
            Err("wrapper did not contain a node identity".to_owned())
        }
    }
}

pub(crate) fn object_is_node_wrapper_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    node_runtime_and_handle_from_object(scope, object).is_ok()
        || receiver_has_detached_state(scope, object)
}

pub(crate) fn node_runtime_and_handle_from_object_or_detached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> std::result::Result<(*mut JsContextHost, DomHandle), String> {
    if let Ok(node) = node_runtime_and_handle_from_object(scope, object) {
        return Ok(node);
    }
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)
        .ok_or_else(|| "missing native bridge host".to_owned())?;
    let handle = super::document::detached_native_handle_for_runtime(scope, runtime_ptr, object)
        .ok_or_else(|| "object did not contain a node identity".to_owned())?;
    Ok((runtime_ptr, handle))
}

pub(crate) fn current_or_live_delegate_node_arg_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    node_or_existing_detached_arg_handle(scope, runtime_ptr, value).or_else(|| {
        let object = v8::Global::new(scope, object);
        let object = v8::Local::new(scope, object);
        live_delegate_arg_handle(scope, runtime_ptr, object)
    })
}

pub(super) fn node_runtime_and_handle_from_args(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> std::result::Result<(*mut JsContextHost, DomHandle), String> {
    node_runtime_and_handle_from_object(scope, args.this())
}

pub(super) fn node_runtime_and_handle_from_args_or_detached(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> std::result::Result<(*mut JsContextHost, DomHandle), String> {
    let this = v8::Global::new(scope, args.this());
    let this = v8::Local::new(scope, this);
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(handle) =
            super::document::detached_native_handle_for_runtime(scope, runtime_ptr, this)
    {
        return Ok((runtime_ptr, handle));
    }
    if let Ok(node) = node_runtime_and_handle_from_object(scope, this) {
        return Ok(node);
    }
    node_runtime_and_handle_from_object_or_detached(scope, this)
}

pub(super) fn node_arg_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    if value.is_null_or_undefined() {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let (object_runtime_ptr, handle) = node_runtime_and_handle_from_object(scope, object).ok()?;
    if object_runtime_ptr != runtime_ptr {
        return None;
    }
    Some(handle)
}

pub(super) fn insertion_document_handle(
    runtime: &JsContextHost,
    parent: DomHandle,
) -> Option<DomHandle> {
    if node_is_document(runtime, parent) {
        Some(parent)
    } else {
        runtime
            .dom_host()
            .node(parent)
            .and_then(Node::owner_document)
    }
}

pub(super) fn node_is_document(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document)
}

pub(super) fn node_is_element(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_element)
}

pub(super) fn set_wrapped_node_or_null(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    handle: Option<DomHandle>,
) {
    set_wrapped_handle_or_null(scope, rv, runtime_ptr, handle);
}
