use super::super::super::{
    document_runtime::DomHandle,
    util::{throw_type_error, v8_string},
};
use super::super::node::{
    node_arg_handle, node_is_document, node_or_foreign_arg_handle_preserve_detached,
    node_runtime_and_handle_from_args_or_detached,
};
use super::super::traversal;
use super::super::{
    JsContextHost, runtime_ptr_from_object, set_wrapped_handle_or_null_for_receiver,
    throw_dom_exception,
};
use super::{
    build_detached_html_collection, build_detached_native_node_list, build_detached_node_list,
    build_object_array, call_object_method, object_dom_identity, object_property_as_object,
    object_property_value, object_string_property,
};

mod detached_selectors;
mod live;
mod traversal_factories;
mod xpath;

pub(in crate::native_bridge) use detached_selectors::{
    bridge_detached_get_element_by_id_callback,
    bridge_detached_get_elements_by_class_name_callback,
    bridge_detached_get_elements_by_name_callback,
    bridge_detached_get_elements_by_tag_name_callback,
    bridge_detached_get_elements_by_tag_name_ns_callback, bridge_detached_matches_callback,
    bridge_detached_query_selector_all_callback, bridge_detached_query_selector_callback,
};
pub(in crate::native_bridge) use live::{
    bridge_document_getter, bridge_get_element_by_id_callback, node_get_element_by_id_callback,
};
pub(in crate::native_bridge) use traversal_factories::{
    node_create_node_iterator_callback, node_create_tree_walker_callback,
};
pub(in crate::native_bridge) use xpath::{
    bridge_detached_document_evaluate_callback, node_document_create_ns_resolver_callback,
    node_document_evaluate_callback,
};
pub(crate) use xpath::{evaluate_live_xpath_search_node_handles, install_xpath_template_bindings};
