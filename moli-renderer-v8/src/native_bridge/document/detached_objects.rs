use super::*;

mod attributes;
mod builders;
mod clone;
mod collections;
mod document_state;
mod method_forwarders;
mod mutation;
mod object_access;
mod prototypes;
mod shadow_dom;
mod state_tree;
mod text_content;

pub(in crate::native_bridge::document) use self::attributes::*;
pub(crate) use self::builders::{
    build_detached_cdata_section_object, build_detached_document_object_from_dom_host,
    build_detached_document_object_from_dom_host_with_content_type, is_valid_pi_target,
    preserve_detached_element_bridge_for_custom_prototype,
};
pub(in crate::native_bridge::document) use self::builders::{
    build_detached_comment_object, build_detached_document_fragment_object,
    build_detached_document_object, build_detached_document_type_object,
    build_detached_element_object, build_detached_html_document_object,
    build_detached_processing_instruction_object, build_detached_text_object,
    copy_detached_element_bridge_members, generic_html_element_proxy,
    html_element_constructor_name, mirror_detached_private_slots, new_detached_state_object,
    new_map_object, remove_detached_element_instance_selector_matching_methods,
    select_html_element_proxy,
};
pub(in crate::native_bridge::document) use self::clone::*;
pub(in crate::native_bridge::document) use self::collections::*;
pub(in crate::native_bridge::document) use self::document_state::*;
pub(crate) use self::method_forwarders::ensure_detached_document_implementation;
pub(in crate::native_bridge) use self::method_forwarders::{
    detached_adopt_node_method_callback, detached_after_method_callback,
    detached_append_child_method_callback, detached_append_method_callback,
    detached_before_method_callback, detached_blur_method_callback, detached_click_method_callback,
    detached_clone_node_method_callback, detached_create_cdata_section_method_callback,
    detached_create_comment_method_callback, detached_create_document_fragment_method_callback,
    detached_create_processing_instruction_method_callback,
    detached_create_text_node_method_callback, detached_focus_method_callback,
    detached_get_attribute_method_callback, detached_get_attribute_names_method_callback,
    detached_get_attribute_ns_method_callback, detached_get_element_by_id_method_callback,
    detached_get_elements_by_class_name_method_callback,
    detached_get_elements_by_name_method_callback,
    detached_get_elements_by_tag_name_method_callback,
    detached_get_elements_by_tag_name_ns_method_callback, detached_has_attribute_method_callback,
    detached_has_attribute_ns_method_callback, detached_has_child_nodes_method_callback,
    detached_import_node_method_callback, detached_insert_before_method_callback,
    detached_matches_method_callback, detached_prepend_method_callback,
    detached_query_selector_all_method_callback, detached_query_selector_method_callback,
    detached_remove_attribute_method_callback, detached_remove_attribute_ns_method_callback,
    detached_remove_child_method_callback, detached_replace_child_method_callback,
    detached_replace_children_method_callback, detached_replace_with_method_callback,
    detached_set_attribute_method_callback, detached_set_attribute_ns_method_callback,
};
pub(in crate::native_bridge::document) use self::method_forwarders::{
    detached_create_cdata_section_html_method_callback,
    detached_create_html_element_method_callback, detached_create_html_element_ns_method_callback,
    detached_create_node_iterator_method_callback, detached_create_xml_element_method_callback,
    detached_create_xml_element_ns_method_callback, detached_document_implementation_getter,
    detached_get_root_node_method_callback, detached_lookup_namespace_uri_method_callback,
    detached_move_before_method_callback, detached_normalize_method_callback,
    detached_remove_method_callback,
};
pub(in crate::native_bridge::document) use self::mutation::*;
pub(in crate::native_bridge::document) use self::object_access::*;
pub(in crate::native_bridge::document) use self::prototypes::*;
pub(in crate::native_bridge::document) use self::shadow_dom::build_detached_shadow_root_object_for_native_handle;
pub(in crate::native_bridge) use self::shadow_dom::{
    detached_attach_shadow_method_callback, detached_shadow_root_active_element_value,
    detached_shadow_root_selection_value,
};
pub(crate) use self::state_tree::detached_is_connected as detached_node_is_connected;
pub(crate) use self::state_tree::{
    DetachedNativeAttributeSnapshot, read_detached_native_attribute,
    read_detached_native_attribute_names, read_detached_native_attribute_snapshot,
    read_detached_native_has_attribute,
    remove_detached_native_attribute_appending_to_current_reaction_queue,
    remove_detached_native_attribute_ns_appending_to_current_reaction_queue,
    write_detached_native_attribute_appending_to_current_reaction_queue,
    write_detached_native_attribute_ns_appending_to_current_reaction_queue,
};
pub(in crate::native_bridge) use self::state_tree::{
    define_detached_native_handle, detached_doctype_name, detached_doctype_public_id,
    detached_doctype_system_id, detached_parent_node_object,
    detached_processing_instruction_target, detached_record_tree_mutation,
    detached_set_owner_document,
};
pub(in crate::native_bridge::document) use self::state_tree::{
    define_detached_state, detached_child_node_objects, detached_detach_for_insert,
    detached_detach_for_insert_appending_to_current_reaction_queue, detached_detach_from_parent,
    detached_detach_from_parent_appending_to_current_reaction_queue,
    detached_element_children_objects, detached_element_sibling_object, detached_has_native_handle,
    detached_is_node, detached_live_delegate_object, detached_native_child_node_objects,
    detached_native_element_runtime_and_handle, detached_native_handle,
    detached_native_mutation_child_node_objects, detached_native_parent_is, detached_node_name,
    detached_node_type, detached_owner_document_object, detached_parent_element_object,
    detached_replace_children_array,
    detached_set_owner_document_appending_to_current_reaction_queue, detached_set_parent,
    detached_sibling_object, detached_state_kind, detached_state_object, detached_state_string,
    detached_tree_query_version, detached_tree_root_object,
    detached_update_existing_children_projection, read_detached_native_text_content,
    sync_detached_native_insert, sync_detached_native_insert_appending_to_current_reaction_queue,
    sync_detached_native_set_attribute, sync_detached_native_set_attribute_ns,
    with_detached_tree_reaction_scope, write_detached_native_text_content,
    write_detached_native_text_content_appending_to_current_reaction_queue,
};
pub(in crate::native_bridge::document) use self::state_tree::{
    detached_is_connected, read_detached_native_attribute_ns, read_detached_native_has_attribute_ns,
};
pub(crate) use self::state_tree::{
    detached_native_handle_for_runtime, detached_native_object_for_handle,
    detached_record_native_tree_mutations, paired_detached_native_object_for_handle,
};
pub(in crate::native_bridge::document) use self::text_content::*;
