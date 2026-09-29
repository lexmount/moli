use super::attributes::{
    bridge_remove_attribute_node_for_live_element_callback,
    bridge_set_attribute_node_for_live_element_callback, detached_create_attribute_method_callback,
    detached_create_attribute_ns_method_callback,
};
use super::*;

mod accessors;
mod bridge_methods;
mod instance_properties;

pub(crate) use accessors::detached_iframe_current_content_document_handle;
pub(in crate::native_bridge) use accessors::{
    clear_detached_iframe_cached_context, clear_detached_iframe_cached_context_for_handle,
    detached_form_owner_object, detached_iframe_content_document, detached_iframe_content_window,
    detached_label_control_object, detached_shadow_root_for_host,
    set_detached_text_replacement_value,
};
pub(in crate::native_bridge) use bridge_methods::install_detached_bridge_methods;
pub(in crate::native_bridge::document) use instance_properties::{
    install_detached_anchor_instance_properties,
    install_detached_character_data_instance_properties,
    install_detached_document_instance_properties,
    install_detached_document_type_instance_properties,
    install_detached_element_instance_properties,
    install_detached_form_associated_instance_properties,
    install_detached_form_control_instance_properties, install_detached_iframe_instance_properties,
    install_detached_image_instance_properties, install_detached_label_instance_properties,
    install_detached_node_core_instance_properties, install_detached_option_instance_properties,
    install_detached_parent_node_instance_properties,
    install_detached_processing_instruction_instance_properties,
    install_detached_select_instance_properties,
    install_detached_text_replacement_instance_properties,
};
