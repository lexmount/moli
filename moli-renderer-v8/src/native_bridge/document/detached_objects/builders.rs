use super::*;

mod character_data;
mod documents;
mod elements;
mod names;
mod nodes;
mod state;

pub(crate) use character_data::build_detached_cdata_section_object;
pub(in crate::native_bridge::document) use character_data::{
    build_detached_comment_object, build_detached_processing_instruction_object,
    build_detached_text_object,
};
pub(in crate::native_bridge::document) use documents::{
    build_detached_document_clone_shell, build_detached_document_object,
    build_detached_html_document_object,
};
pub(crate) use documents::{
    build_detached_document_object_from_dom_host,
    build_detached_document_object_from_dom_host_with_content_type,
};
pub(crate) use elements::preserve_detached_element_bridge_for_custom_prototype;
pub(in crate::native_bridge::document) use elements::{
    build_detached_element_object, copy_detached_element_bridge_members,
    generic_html_element_proxy, mirror_detached_private_slots,
    remove_detached_element_instance_selector_matching_methods, select_html_element_proxy,
};
pub(crate) use names::is_valid_pi_target;
pub(in crate::native_bridge::document) use names::{
    html_element_constructor_name, html_element_to_string_tag, qualified_name_parts,
    svg_element_constructor_name, svg_element_to_string_tag,
};
pub(in crate::native_bridge::document) use nodes::{
    build_detached_document_fragment_object, build_detached_document_type_object,
};
pub(in crate::native_bridge::document) use state::{new_detached_state_object, new_map_object};

fn initialize_new_detached_native_node_owner_document(
    runtime_ptr: *mut JsContextHost,
    owner_document: DomHandle,
    handle: DomHandle,
) -> Option<DomHandle> {
    // This is creation-time owner assignment for a freshly created native node
    // backing a detached wrapper. It is not user-visible adoption and must not
    // dispatch adoptedCallback.
    unsafe { &mut *runtime_ptr }.initialize_new_native_node_owner_document(owner_document, handle)
}
