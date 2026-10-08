mod clone_adopt;
mod document_methods;
mod dom_implementation;
mod global_factories;

pub(in crate::native_bridge) use self::clone_adopt::{
    bridge_adopt_node_into_document_callback, bridge_clone_node_into_document_callback,
    bridge_detached_clone_node_callback,
};
pub(in crate::native_bridge::document) use self::document_methods::create_detached_element_for_document;
pub(in crate::native_bridge) use self::document_methods::{
    bridge_create_cdata_section_not_supported_callback,
    bridge_detached_create_cdata_section_callback, bridge_detached_create_comment_callback,
    bridge_detached_create_document_fragment_callback,
    bridge_detached_create_processing_instruction_callback, bridge_detached_create_text_callback,
};
pub(in crate::native_bridge) use self::dom_implementation::{
    bridge_create_detached_document_callback, bridge_create_detached_document_type_callback,
    bridge_create_detached_html_document_callback, bridge_create_detached_xml_document_callback,
};
pub(in crate::native_bridge) use self::global_factories::{
    bridge_create_detached_comment_callback, bridge_create_detached_document_fragment_callback,
    bridge_create_detached_text_callback,
};
