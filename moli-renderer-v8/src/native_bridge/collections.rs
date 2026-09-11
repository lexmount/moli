use crate::web_api_interfaces;
use std::ffi::c_void;

use crate::dom::{forms::InputType, native::Node};

use super::super::{
    document_runtime::DomHandle,
    util::{get_private_value, set_private_value, v8_string},
};
use super::bindings::set_named_constructor_prototype;
use super::element::{
    resize_select_options, select_add_insertion_point, select_options_resize_target,
    set_select_indexed_option,
};
use super::identity::{CollectionKind, LiveCollectionDescriptor, LiveCollectionQueryKind};
use super::{
    JsContextHost,
    bridge::wrapped_handle_value,
    callback_arg_dom_handle, callback_arg_namespace, callback_arg_optional_string,
    callback_arg_string, callback_value_dom_handle, current_or_live_delegate_node_arg_handle,
    node::{
        append_child_in_reaction_scope, insert_before_in_reaction_scope,
        remove_child_in_reaction_scope,
    },
    runtime_ptr_from_object, set_wrapped_handle_array,
};

mod bridge_callbacks;
mod builders;
mod iteration;
mod live_handlers;
mod options_collection;
mod radio_node_list;
mod shared;
mod templates;

pub(super) use bridge_callbacks::{
    bridge_create_html_collection_callback, bridge_create_live_html_collection_callback,
    bridge_create_live_node_list_callback, bridge_create_node_list_callback,
    bridge_get_elements_by_class_name_callback, bridge_get_elements_by_name_callback,
    bridge_get_elements_by_tag_name_callback, bridge_get_elements_by_tag_name_ns_callback,
    bridge_resolve_live_collection_callback,
};
pub(in crate::native_bridge::collections) use builders::STATIC_HANDLE_COLLECTION_ID_INTERNAL_FIELD;
pub(crate) use builders::install_collection_template_bindings;
pub(in crate::native_bridge) use builders::{
    STATIC_COLLECTION_LENGTH_SLOT, build_fresh_document_all_named_collection_value,
    is_document_all_named_collection_value,
};
pub(super) use builders::{
    build_collection_wrapper, build_live_child_node_list_for_node, build_live_collection_for_node,
    build_live_collection_wrapper, build_live_html_children_collection_for_node,
    build_node_list_from_handles,
};
pub(in crate::native_bridge) use shared::array_index_property_name;
// Keep these re-exports visible only inside `collections`; sibling modules use `super::*`
// without leaking collection-private helpers to the broader native_bridge surface.
pub(in crate::native_bridge::collections) use iteration::*;
pub(in crate::native_bridge::collections) use live_handlers::*;
pub(in crate::native_bridge::collections) use options_collection::*;
pub(in crate::native_bridge::collections) use radio_node_list::*;
pub(in crate::native_bridge::collections) use shared::*;
pub(super) use templates::{
    build_collection_wrapper_template, build_live_collection_wrapper_template,
    build_static_handle_node_list_wrapper_template,
};

pub(in crate::native_bridge::collections) fn collection_kind_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CollectionKind> {
    match moli_webapi_declare::web_api_object_type(scope, object)?.name() {
        "NodeList" => Some(CollectionKind::NodeList),
        "HTMLCollection" => Some(CollectionKind::HtmlCollection),
        "HTMLFormControlsCollection" => Some(CollectionKind::FormControlsCollection),
        "HTMLOptionsCollection" => Some(CollectionKind::OptionsCollection),
        "RadioNodeList" => Some(CollectionKind::RadioNodeList),
        _ => None,
    }
}

pub(in crate::native_bridge) fn initialize_collection_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    kind: CollectionKind,
) {
    let interface = collection_interface_name(kind);
    web_api_interfaces::initialize(scope, object, interface)
        .expect("native collection identity should initialize");
}

pub(in crate::native_bridge::collections) fn is_node_list_kind(kind: CollectionKind) -> bool {
    matches!(
        kind,
        CollectionKind::NodeList | CollectionKind::RadioNodeList
    )
}

pub(in crate::native_bridge::collections) fn is_html_collection_kind(kind: CollectionKind) -> bool {
    matches!(
        kind,
        CollectionKind::HtmlCollection
            | CollectionKind::FormControlsCollection
            | CollectionKind::OptionsCollection
    )
}

fn collection_interface_name(kind: CollectionKind) -> &'static str {
    match kind {
        CollectionKind::NodeList => "NodeList",
        CollectionKind::HtmlCollection => "HTMLCollection",
        CollectionKind::FormControlsCollection => "HTMLFormControlsCollection",
        CollectionKind::OptionsCollection => "HTMLOptionsCollection",
        CollectionKind::RadioNodeList => "RadioNodeList",
    }
}
