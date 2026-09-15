use super::*;

mod attributes;
mod builders;
mod clone;
mod collections;
mod document_state;
mod equality;
mod method_forwarders;
mod mutation;
mod object_access;
mod prototypes;
mod shadow_dom;
mod state_tree;

pub(in crate::native_bridge::document) use self::attributes::*;
pub(in crate::native_bridge::document) use self::builders::*;
pub(crate) use self::builders::{
    build_detached_cdata_section_object, build_detached_document_object_from_dom_host,
    build_detached_document_object_from_dom_host_with_content_type, is_valid_pi_target,
    preserve_detached_element_bridge_for_custom_prototype,
};
pub(in crate::native_bridge::document) use self::clone::*;
pub(in crate::native_bridge::document) use self::collections::*;
pub(in crate::native_bridge::document) use self::document_state::*;
pub(in crate::native_bridge::document) use self::equality::*;
pub(crate) use self::method_forwarders::ensure_detached_document_implementation;
pub(in crate::native_bridge) use self::method_forwarders::*;
pub(in crate::native_bridge::document) use self::mutation::*;
pub(in crate::native_bridge::document) use self::object_access::*;
pub(in crate::native_bridge::document) use self::prototypes::*;
pub(in crate::native_bridge::document) use self::shadow_dom::*;
pub(in crate::native_bridge) use self::shadow_dom::{
    detached_attach_shadow_method_callback, detached_shadow_root_active_element_value,
    detached_shadow_root_selection_value,
};
pub(crate) use self::state_tree::detached_is_connected as detached_node_is_connected;
pub(in crate::native_bridge::document) use self::state_tree::*;
pub(crate) use self::state_tree::{
    DetachedNativeAttributeSnapshot, read_detached_native_attribute,
    read_detached_native_attribute_names, read_detached_native_attribute_snapshot,
    read_detached_native_has_attribute,
    remove_detached_native_attribute_appending_to_current_reaction_queue,
    remove_detached_native_attribute_ns_appending_to_current_reaction_queue,
    with_detached_native_element_reaction_scope,
    write_detached_native_attribute_appending_to_current_reaction_queue,
    write_detached_native_attribute_ns_appending_to_current_reaction_queue,
};
pub(in crate::native_bridge) use self::state_tree::{
    define_detached_native_handle, detached_doctype_name, detached_doctype_public_id,
    detached_doctype_system_id, detached_parent_node_object,
    detached_processing_instruction_target, detached_record_tree_mutation,
    detached_set_owner_document,
};
pub(crate) use self::state_tree::{
    detached_native_handle_for_runtime, detached_native_object_for_handle,
    paired_detached_native_object_for_handle,
};
