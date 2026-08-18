use crate::{
    custom_elements,
    dom::native::{DocumentTitleSetterTarget, Node},
    native_bridge::abort::dom_exception_value,
};

use super::super::{
    document_runtime::DomHandle,
    util::{
        call_global_bridge_method, context_host_ptr_from_global_bridge, get_private_value,
        global_bridge_object, set_private_value, throw_type_error, v8_string, v8str,
    },
};
use super::node::{
    node_is_document, node_runtime_and_handle_from_object,
    node_runtime_and_handle_from_object_or_detached,
};
use super::{
    JsContextHost, callback_arg_namespace, callback_arg_string, collections,
    identity::{CollectionKind, LiveCollectionDescriptor, LiveCollectionQueryKind},
    runtime_ptr_from_object, set_wrapped_handle_or_null, throw_dom_exception,
    validate_attribute_name, validate_element_name, validate_qualified_element_name_and_namespace,
    validate_qualified_name_and_namespace,
};
use moli_webapi_declare::WebApiFunctionTemplate;

mod attributes;
mod construction;
mod cookies;
mod css_state;
mod custom_element_options;
mod detached_install;
mod detached_objects;
mod detached_surface;
mod hit_test;
mod lifecycle;
mod live_collections;
mod queries;
mod state;
mod structure;

pub(crate) use attributes::install_named_node_map_template_bindings;
pub(crate) use queries::evaluate_live_xpath_search_node_handles;

pub(crate) const DETACHED_STATE_SLOT: &str = "__moliDetachedState";
pub(crate) const DETACHED_LIVE_DELEGATE_SLOT: &str = "__moliLiveDelegate";
pub(crate) const DETACHED_NATIVE_HANDLE_SLOT: &str = "__moliDetachedNativeHandle";
pub(crate) const DETACHED_NATIVE_NODE_LIST_HANDLES_SLOT: &str =
    "__moliDetachedNativeNodeListHandles";
const DOCUMENT_ASSOCIATED_WINDOW_SLOT: &str = "__moliDocumentAssociatedWindow";

pub(crate) fn node_document_design_mode_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_document(runtime, handle) {
        return;
    }
    let value = if runtime.document_design_mode_enabled(handle) {
        "on"
    } else {
        "off"
    };
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    }
}

pub(crate) fn node_document_design_mode_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    let enabled = if value.eq_ignore_ascii_case("on") {
        true
    } else if value.eq_ignore_ascii_case("off") {
        false
    } else {
        return;
    };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if node_is_document(runtime, handle) {
        runtime.set_document_design_mode_enabled(handle, enabled);
    }
}

pub(in crate::native_bridge) use attributes::is_attr_node_value;
pub(super) use attributes::{
    build_named_node_map_wrapper_template, clear_live_attr_cache_entry,
    clear_live_attr_cache_entry_ns, live_get_attribute_node_ns_object,
};
use attributes::{
    detached_create_attribute_method_callback, detached_create_attribute_ns_method_callback,
};
pub(in crate::native_bridge) use attributes::{
    detached_get_attribute_node_method_callback, detached_get_attribute_node_ns_method_callback,
    detached_remove_attribute_node_method_callback, detached_set_attribute_node_method_callback,
};
use attributes::{live_attr_cache_object, namespace_attr_cache_key, set_attr_cache_entry};
pub(crate) use attributes::{
    live_get_attribute_node_object, live_named_node_map_wrapper, new_attr_object,
};
pub(super) use construction::{
    bridge_create_comment_callback, bridge_create_element_callback,
    bridge_create_element_ns_callback, bridge_create_processing_instruction_callback,
    bridge_create_text_node_callback, node_adopt_node_callback, node_create_attribute_callback,
    node_create_attribute_ns_callback, node_create_cdata_section_callback,
    node_create_comment_callback, node_create_document_fragment_callback,
    node_create_element_callback, node_create_element_ns_callback,
    node_create_processing_instruction_callback, node_create_text_node_callback,
    node_import_node_callback,
};
pub(crate) use cookies::{document_cookie_for_receiver, set_document_cookie_for_receiver};
pub(crate) use css_state::install_adopted_style_sheets_array_primordials;
pub(in crate::native_bridge) use css_state::{
    AdoptedStyleSheetsArrayOwner, install_adopted_style_sheets_array_mutation_methods,
    normalize_adopted_style_sheets_assignment,
};
pub(crate) use css_state::{
    apply_stylesheet_owner_css_projections, apply_stylesheet_source_css_projection,
    clear_adopted_stylesheet_font_face_wrappers, sync_document_fonts_for_handle,
};
use css_state::{detached_document_fonts_getter, document_fonts_getter_function};
pub(crate) use css_state::{
    node_document_adopted_style_sheets_getter_function,
    node_document_adopted_style_sheets_setter_function, node_document_style_sheets_getter_function,
};
pub(in crate::native_bridge::document) use custom_element_options::parse_import_node_options;
pub(in crate::native_bridge) use custom_element_options::validate_registry_association_for_document;
pub(crate) use detached_install::detached_iframe_current_content_document_handle;
pub(super) use detached_install::install_detached_bridge_methods;
pub(in crate::native_bridge) use detached_install::{
    clear_detached_iframe_cached_context, clear_detached_iframe_cached_context_for_handle,
    detached_form_owner_object, detached_form_reset_callback, detached_form_submit_callback,
    detached_iframe_content_document, detached_iframe_content_window,
    detached_label_control_object, detached_shadow_root_for_host, set_detached_node_text_content,
    set_detached_text_replacement_value,
};
use detached_install::{
    detached_document_anchors_value, detached_document_applets_value,
    detached_document_embeds_value, detached_document_forms_value, detached_document_images_value,
    detached_document_links_value, detached_document_scripts_value,
    install_detached_character_data_instance_properties,
    install_detached_document_instance_properties,
    install_detached_document_type_instance_properties,
    install_detached_element_instance_properties, install_detached_node_core_instance_properties,
    install_detached_processing_instruction_instance_properties,
};
pub(in crate::native_bridge::document) use detached_objects::*;
pub(crate) use detached_objects::{
    build_detached_cdata_section_object, build_detached_document_object_from_dom_host,
    build_detached_document_object_from_dom_host_with_content_type, detached_node_is_connected,
    is_valid_pi_target, preserve_detached_element_bridge_for_custom_prototype,
    read_detached_native_attribute, read_detached_native_attribute_names,
    read_detached_native_attribute_snapshot, read_detached_native_has_attribute,
    remove_detached_native_attribute_appending_to_current_reaction_queue,
    remove_detached_native_attribute_ns_appending_to_current_reaction_queue,
    with_detached_native_element_reaction_scope,
    write_detached_native_attribute_appending_to_current_reaction_queue,
    write_detached_native_attribute_ns_appending_to_current_reaction_queue,
};
pub(in crate::native_bridge) use detached_objects::{
    define_detached_native_handle, detached_append_child_method_callback,
    detached_clone_node_method_callback, detached_doctype_name, detached_doctype_public_id,
    detached_doctype_system_id, detached_insert_before_method_callback,
    detached_parent_node_object, detached_processing_instruction_target,
    detached_remove_child_method_callback, detached_replace_child_method_callback,
    detached_set_owner_document,
};
pub(in crate::native_bridge) use detached_objects::{
    detached_attach_shadow_method_callback, detached_blur_method_callback,
    detached_click_method_callback, detached_focus_method_callback,
};
pub(in crate::native_bridge) use detached_objects::{
    detached_get_attribute_method_callback, detached_get_attribute_names_method_callback,
    detached_get_attribute_ns_method_callback, detached_get_elements_by_class_name_method_callback,
    detached_get_elements_by_name_method_callback,
    detached_get_elements_by_tag_name_method_callback,
    detached_get_elements_by_tag_name_ns_method_callback, detached_has_attribute_method_callback,
    detached_has_attribute_ns_method_callback, detached_matches_method_callback,
    detached_remove_attribute_method_callback, detached_remove_attribute_ns_method_callback,
    detached_set_attribute_method_callback, detached_set_attribute_ns_method_callback,
};
pub(crate) use detached_objects::{
    detached_native_handle_for_runtime, detached_native_object_for_handle,
    paired_detached_native_object_for_handle,
};
pub(in crate::native_bridge) use detached_objects::{
    detached_query_selector_all_method_callback, detached_query_selector_method_callback,
};
pub(in crate::native_bridge) use detached_objects::{
    detached_shadow_root_active_element_value, detached_shadow_root_selection_value,
};
pub(in crate::native_bridge::document) use detached_surface::detached_document_content_type_value;
pub(super) use detached_surface::{
    bridge_adopt_node_into_document_callback, bridge_clone_node_into_document_callback,
    bridge_create_cdata_section_not_supported_callback, bridge_create_detached_comment_callback,
    bridge_create_detached_document_callback, bridge_create_detached_document_fragment_callback,
    bridge_create_detached_document_type_callback, bridge_create_detached_html_document_callback,
    bridge_create_detached_text_callback, bridge_create_detached_xml_document_callback,
    bridge_detached_after_callback, bridge_detached_append_callback,
    bridge_detached_append_child_callback, bridge_detached_before_callback,
    bridge_detached_character_data_getter_callback, bridge_detached_character_data_setter_callback,
    bridge_detached_child_element_count_callback, bridge_detached_child_nodes_callback,
    bridge_detached_children_callback, bridge_detached_clone_node_callback,
    bridge_detached_contains_callback, bridge_detached_create_cdata_section_callback,
    bridge_detached_create_comment_callback, bridge_detached_create_document_fragment_callback,
    bridge_detached_create_element_callback,
    bridge_detached_create_processing_instruction_callback, bridge_detached_create_text_callback,
    bridge_detached_doctype_name_callback, bridge_detached_doctype_public_id_callback,
    bridge_detached_doctype_system_id_callback, bridge_detached_document_base_uri_callback,
    bridge_detached_document_body_callback, bridge_detached_document_body_setter_callback,
    bridge_detached_document_character_set_callback, bridge_detached_document_compat_mode_callback,
    bridge_detached_document_content_type_callback, bridge_detached_document_doctype_callback,
    bridge_detached_document_domain_callback, bridge_detached_document_element_callback,
    bridge_detached_document_head_callback, bridge_detached_document_ready_state_callback,
    bridge_detached_document_referrer_callback, bridge_detached_document_title_getter_callback,
    bridge_detached_document_title_setter_callback, bridge_detached_document_uri_callback,
    bridge_detached_document_url_callback, bridge_detached_element_local_name_callback,
    bridge_detached_element_namespace_uri_callback, bridge_detached_element_prefix_callback,
    bridge_detached_element_tag_name_callback, bridge_detached_first_child_callback,
    bridge_detached_first_element_child_callback, bridge_detached_get_attribute_callback,
    bridge_detached_get_attribute_names_callback, bridge_detached_get_attribute_ns_callback,
    bridge_detached_has_attribute_callback, bridge_detached_has_attribute_ns_callback,
    bridge_detached_has_child_nodes_callback, bridge_detached_insert_before_callback,
    bridge_detached_is_connected_callback, bridge_detached_is_equal_node_callback,
    bridge_detached_is_same_node_callback, bridge_detached_last_child_callback,
    bridge_detached_last_element_child_callback, bridge_detached_move_before_callback,
    bridge_detached_next_element_sibling_callback, bridge_detached_next_sibling_callback,
    bridge_detached_node_name_callback, bridge_detached_node_type_callback,
    bridge_detached_node_value_getter_callback, bridge_detached_node_value_setter_callback,
    bridge_detached_owner_document_callback, bridge_detached_parent_element_callback,
    bridge_detached_parent_node_callback, bridge_detached_prepend_callback,
    bridge_detached_previous_element_sibling_callback, bridge_detached_previous_sibling_callback,
    bridge_detached_processing_instruction_target_callback,
    bridge_detached_remove_attribute_callback, bridge_detached_remove_attribute_ns_callback,
    bridge_detached_remove_child_callback, bridge_detached_replace_child_callback,
    bridge_detached_replace_children_callback, bridge_detached_replace_with_callback,
    bridge_detached_set_attribute_callback, bridge_detached_set_attribute_ns_callback,
    bridge_detached_text_content_callback, bridge_set_detached_document_domain_callback,
    detached_character_data_append_data_callback, detached_character_data_delete_data_callback,
    detached_character_data_insert_data_callback, detached_character_data_length,
    detached_character_data_replace_data_callback, detached_character_data_substring_data_callback,
    detached_character_data_value, detached_text_split_text_callback,
    detached_text_whole_text_value, set_detached_character_data_value,
};
pub(in crate::native_bridge) use detached_surface::{
    detached_element_local_name, detached_element_namespace_uri, detached_element_prefix,
};
pub(crate) use hit_test::install_caret_position_template_bindings;
pub(super) use hit_test::{
    node_document_caret_position_from_point_callback, node_document_element_from_point_callback,
    node_document_elements_from_point_callback, node_shadow_root_element_from_point_callback,
    node_shadow_root_elements_from_point_callback,
};
pub(in crate::native_bridge) use lifecycle::{
    append_detached_html_document_body_html, set_detached_html_document_body_html,
};
pub(super) use lifecycle::{
    node_document_close_callback, node_document_exec_command_callback, node_document_open_callback,
    node_document_query_command_enabled_callback, node_document_query_command_indeterm_callback,
    node_document_query_command_state_callback, node_document_query_command_supported_callback,
    node_document_query_command_value_callback, node_document_write_callback,
    node_document_writeln_callback,
};
pub(crate) use live_collections::document_all_value_for_receiver;
pub(crate) use queries::install_xpath_template_bindings;
pub(super) use queries::{
    bridge_detached_document_evaluate_callback, bridge_detached_get_element_by_id_callback,
    bridge_detached_get_elements_by_class_name_callback,
    bridge_detached_get_elements_by_name_callback,
    bridge_detached_get_elements_by_tag_name_callback,
    bridge_detached_get_elements_by_tag_name_ns_callback, bridge_detached_matches_callback,
    bridge_detached_query_selector_all_callback, bridge_detached_query_selector_callback,
    bridge_document_getter, bridge_get_element_by_id_callback, node_create_node_iterator_callback,
    node_create_tree_walker_callback, node_document_create_ns_resolver_callback,
    node_document_evaluate_callback, node_get_element_by_id_callback,
};
pub(super) use state::{
    node_document_active_element_getter_function, throw_document_domain_security_error,
};
pub(in crate::native_bridge::document) use structure::set_document_body_for_native_handle_appending_to_current_reaction_queue;

pub(crate) const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
pub(crate) const SVG_NS: &str = "http://www.w3.org/2000/svg";

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document", enumerable)]
struct DocumentMetadataPrototypeDeclaration {
    #[webapi(accessor_property = "URL", getter = document_url_getter_function)]
    url: (),
    #[webapi(accessor_property = "documentURI", getter = document_uri_getter_function)]
    document_uri: (),
    #[webapi(accessor_property = "readyState", getter = document_ready_state_getter_function)]
    ready_state: (),
    #[webapi(accessor_property = "contentType", getter = document_content_type_getter_function)]
    content_type: (),
    #[webapi(accessor_property = "characterSet", getter = document_character_set_getter_function)]
    character_set: (),
    #[webapi(accessor_property, getter = document_character_set_getter_function)]
    charset: (),
    #[webapi(accessor_property = "inputEncoding", getter = document_character_set_getter_function)]
    input_encoding: (),
    #[webapi(accessor_property = "compatMode", getter = document_compat_mode_getter_function)]
    compat_mode: (),
    #[webapi(
        accessor_property = "lastModified",
        getter = document_last_modified_getter_function
    )]
    last_modified: (),
    #[webapi(accessor_property, getter = document_referrer_getter_function)]
    referrer: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document", enumerable)]
struct DocumentStructurePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = document_title_getter_function,
        setter = document_title_setter_function
    )]
    title: (),
    #[webapi(
        accessor_property = "documentElement",
        getter = document_document_element_getter_function
    )]
    document_element: (),
    #[webapi(accessor_property, getter = document_doctype_getter_function)]
    doctype: (),
    #[webapi(accessor_property, getter = document_head_getter_function)]
    head: (),
    #[webapi(
        accessor_property,
        getter = document_body_getter_function,
        setter = document_body_setter_function
    )]
    body: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document", enumerable)]
struct DocumentViewPrototypeDeclaration {
    #[webapi(
        accessor_property = "defaultView",
        getter = document_default_view_getter_function
    )]
    default_view: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document", enumerable)]
struct DocumentFocusPrototypeDeclaration {
    #[webapi(
        accessor_property = "activeElement",
        getter = node_document_active_element_getter_function
    )]
    active_element: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document", enumerable)]
struct DocumentStatePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = detached_document_implementation_getter
    )]
    implementation: (),
    #[webapi(accessor_property, getter = document_fonts_getter_function)]
    fonts: (),
    #[webapi(
        accessor_property = "currentScript",
        getter = document_current_script_getter_function
    )]
    current_script: (),
    #[webapi(accessor_property, getter = document_hidden_getter_function)]
    hidden: (),
    #[webapi(
        accessor_property = "visibilityState",
        getter = document_visibility_state_getter_function
    )]
    visibility_state: (),
    #[webapi(accessor_property, getter = document_prerendering_getter_function)]
    prerendering: (),
    #[webapi(
        accessor_property,
        getter = document_domain_getter_function,
        setter = document_domain_setter_function
    )]
    domain: (),
    #[webapi(
        accessor_property = "scrollingElement",
        getter = document_scrolling_element_getter_function
    )]
    scrolling_element: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document", enumerable)]
struct DocumentCollectionAccessorsPrototypeDeclaration {
    #[webapi(accessor_property, getter = document_forms_getter_function)]
    forms: (),
    #[webapi(accessor_property, getter = document_images_getter_function)]
    images: (),
    #[webapi(accessor_property, getter = document_scripts_getter_function)]
    scripts: (),
    #[webapi(accessor_property, getter = document_links_getter_function)]
    links: (),
    #[webapi(accessor_property, getter = document_anchors_getter_function)]
    anchors: (),
    #[webapi(accessor_property, getter = document_embeds_getter_function)]
    embeds: (),
    #[webapi(accessor_property, getter = document_plugins_getter_function)]
    plugins: (),
    #[webapi(accessor_property, getter = document_applets_getter_function)]
    applets: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document")]
struct DocumentCollectionQueryPrototypeDeclaration {
    #[webapi(
        method,
        length = 1,
        callback = super::element::node_get_elements_by_tag_name_callback
    )]
    get_elements_by_tag_name: (),
    #[webapi(
        method = "getElementsByTagNameNS",
        length = 2,
        callback = super::element::node_get_elements_by_tag_name_ns_callback
    )]
    get_elements_by_tag_name_ns: (),
    #[webapi(
        method,
        length = 1,
        callback = super::element::node_get_elements_by_class_name_callback
    )]
    get_elements_by_class_name: (),
    #[webapi(
        method,
        length = 1,
        callback = super::element::node_get_elements_by_name_callback
    )]
    get_elements_by_name: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DocumentFragment")]
struct DocumentFragmentCollectionQueryPrototypeDeclaration {
    #[webapi(method, length = 1, callback = node_get_element_by_id_callback)]
    get_element_by_id: (),
    #[webapi(
        method,
        length = 1,
        callback = super::element::node_get_elements_by_tag_name_callback
    )]
    get_elements_by_tag_name: (),
    #[webapi(
        method = "getElementsByTagNameNS",
        length = 2,
        callback = super::element::node_get_elements_by_tag_name_ns_callback
    )]
    get_elements_by_tag_name_ns: (),
    #[webapi(
        method,
        length = 1,
        callback = super::element::node_get_elements_by_class_name_callback
    )]
    get_elements_by_class_name: (),
    #[webapi(
        method,
        length = 1,
        callback = super::element::node_get_elements_by_name_callback
    )]
    get_elements_by_name: (),
}

fn document_receiver_runtime_and_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return None;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        return None;
    }
    Some((runtime_ptr, handle))
}

fn set_document_string_return_value(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn set_document_node_return_value_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    runtime_ptr: *mut JsContextHost,
    receiver: v8::Local<'s, v8::Object>,
    handle: Option<DomHandle>,
) {
    let Some(handle) = handle else {
        rv.set_null();
        return;
    };
    if detached_native_handle_for_runtime(scope, runtime_ptr, receiver).is_some() {
        match detached_native_object_for_handle(scope, runtime_ptr, handle) {
            Some(node) => rv.set(node.into()),
            None => rv.set_null(),
        }
    } else {
        set_wrapped_handle_or_null(scope, rv, runtime_ptr, Some(handle));
    }
}

fn document_title_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let title = runtime.dom_host().dom().document_title_for_document(handle);
    set_document_string_return_value(scope, &mut rv, &title);
}

pub(in crate::native_bridge) fn set_document_title_for_handle_appending_to_current_reaction_queue(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    title_text: &str,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    if !node_is_document(runtime, handle) {
        return;
    }
    let Some(target) = runtime
        .dom_host()
        .dom()
        .document_title_setter_target_for_document(handle)
    else {
        return;
    };
    let title_handle = match target {
        DocumentTitleSetterTarget::ExistingTitle(title) => title,
        DocumentTitleSetterTarget::AppendToHtmlHead(head) => {
            let title = runtime.create_element("title");
            if runtime.dom_host().owner_document_handle(title) != Some(handle)
                && runtime
                    .initialize_new_native_node_owner_document(handle, title)
                    .is_none()
            {
                return;
            }
            if !runtime.append_child_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                head,
                title,
            ) {
                return;
            }
            title
        }
        DocumentTitleSetterTarget::PrependToSvgRoot(root) => {
            let Some(title) = runtime.create_element_ns(Some(SVG_NS), "title") else {
                return;
            };
            if runtime.dom_host().owner_document_handle(title) != Some(handle)
                && runtime
                    .initialize_new_native_node_owner_document(handle, title)
                    .is_none()
            {
                return;
            }
            let first_child = runtime.dom_host().first_child(root);
            if !runtime.insert_before_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                root,
                title,
                first_child,
            ) {
                return;
            }
            title
        }
    };

    let _ = runtime.set_text_content_appending_to_current_reaction_queue(
        scope,
        runtime_ptr,
        title_handle,
        title_text,
    );
}

fn document_title_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = args.get(0).to_string(scope) else {
        rv.set_undefined();
        return;
    };
    let title_text = value.to_rust_string_lossy(scope);

    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        set_document_title_for_handle_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            &title_text,
        );
    });
    rv.set_undefined();
}

fn document_document_element_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host().dom();
    let element = dom
        .node(handle)
        .and_then(Node::as_document)
        .and_then(|document| document.document_element_handle(dom, handle));
    set_document_node_return_value_for_receiver(scope, &mut rv, runtime_ptr, receiver, element);
}

fn document_doctype_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let doctype = runtime.dom_host().child_nodes(handle).and_then(|children| {
        children.into_iter().find(|child| {
            runtime
                .dom_host()
                .node(*child)
                .and_then(Node::as_document_type)
                .is_some()
        })
    });
    set_document_node_return_value_for_receiver(scope, &mut rv, runtime_ptr, receiver, doctype);
}

fn document_head_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host().dom();
    let head = dom
        .node(handle)
        .and_then(Node::as_document)
        .and_then(|document| document.head_handle(dom, handle));
    set_document_node_return_value_for_receiver(scope, &mut rv, runtime_ptr, receiver, head);
}

fn document_body_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host().dom();
    let body = dom
        .node(handle)
        .and_then(Node::as_document)
        .and_then(|document| document.body_or_frameset_handle(dom, handle));
    set_document_node_return_value_for_receiver(scope, &mut rv, runtime_ptr, receiver, body);
}

fn document_body_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, args.this())
    else {
        throw_type_error(
            scope,
            "Failed to set 'body' on 'Document': Illegal invocation.",
        );
        return;
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let _ = set_document_body_for_native_handle_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            handle,
            args.get(0),
        );
    });
    rv.set_undefined();
}

fn document_url_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    if let Some(url) = detached_state_string(scope, receiver, "url") {
        set_document_string_return_value(scope, &mut rv, &url);
        return;
    }
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_undefined();
        return;
    };
    let url = unsafe { &*runtime_ptr }
        .document_url_for_handle(handle)
        .to_string();
    set_document_string_return_value(scope, &mut rv, &url);
}

fn document_uri_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    if let Some(url) = detached_state_string(scope, receiver, "documentURI") {
        set_document_string_return_value(scope, &mut rv, &url);
        return;
    }
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_undefined();
        return;
    };
    let url = unsafe { &*runtime_ptr }
        .document_url_for_handle(handle)
        .to_string();
    set_document_string_return_value(scope, &mut rv, &url);
}

fn document_ready_state_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    if let Some(ready_state) = detached_state_string(scope, receiver, "readyState") {
        set_document_string_return_value(scope, &mut rv, &ready_state);
        return;
    }
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_undefined();
        return;
    };
    let ready_state = unsafe { &*runtime_ptr }.document_ready_state_for_handle(handle);
    set_document_string_return_value(scope, &mut rv, &ready_state);
}

fn document_content_type_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    runtime: &JsContextHost,
    handle: DomHandle,
) -> String {
    if let Some(content_type) =
        detached_state_string(scope, receiver, "contentType").filter(|value| !value.is_empty())
    {
        return content_type;
    }
    let Some(document) = runtime.dom_host().node(handle).and_then(Node::as_document) else {
        return runtime
            .dom_host()
            .document_content_type_for_handle(handle)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "application/xml".to_owned());
    };
    if document.is_html_document() {
        let content_type = document.content_type();
        return if content_type.is_empty() {
            "text/html".to_owned()
        } else {
            content_type.to_owned()
        };
    }
    if let Some(creation_namespace) = detached_state_string(scope, receiver, "creationNamespace")
        .filter(|value| !value.is_empty())
    {
        if creation_namespace == XHTML_NS {
            return "application/xhtml+xml".to_owned();
        }
        if creation_namespace == SVG_NS {
            return "image/svg+xml".to_owned();
        }
    }
    if let Some(content_type) = runtime.dom_host().document_content_type_for_handle(handle) {
        return content_type.to_owned();
    }
    let root_namespace = document
        .document_element_handle(runtime.dom_host().dom(), handle)
        .and_then(|root| runtime.dom_host().node(root))
        .and_then(Node::as_element)
        .map(|element| element.namespace().to_owned());
    if root_namespace.as_deref() == Some(XHTML_NS) {
        "application/xhtml+xml".to_owned()
    } else if root_namespace.as_deref() == Some(SVG_NS) {
        "image/svg+xml".to_owned()
    } else {
        runtime
            .dom_host()
            .document_content_type_for_handle(handle)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "application/xml".to_owned())
    }
}

fn document_content_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let content_type = document_content_type_for_receiver(scope, receiver, runtime, handle);
    set_document_string_return_value(scope, &mut rv, &content_type);
}

fn document_character_set_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    if detached_state_object(scope, receiver).is_some() {
        let character_set = detached_state_string(scope, receiver, "characterSet")
            .unwrap_or_else(|| "UTF-8".into());
        set_document_string_return_value(scope, &mut rv, &character_set);
        return;
    }
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let character_set = runtime
        .child_browsing_context_character_set_for_document_handle(handle)
        .unwrap_or_else(|| runtime.document_character_set())
        .to_owned();
    set_document_string_return_value(scope, &mut rv, &character_set);
}

fn document_compat_mode_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let compat_mode = unsafe { &*runtime_ptr }
        .dom_host()
        .document_quirks_mode_for_handle(handle)
        .map(|mode| match mode {
            selectors::matching::QuirksMode::Quirks => "BackCompat",
            selectors::matching::QuirksMode::LimitedQuirks
            | selectors::matching::QuirksMode::NoQuirks => "CSS1Compat",
        })
        .unwrap_or("CSS1Compat");
    set_document_string_return_value(scope, &mut rv, compat_mode);
}

fn document_last_modified_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = moli_time::format_document_last_modified_value(
        runtime
            .dom_host()
            .document_source_last_modified_for_handle(handle),
        moli_time::unix_epoch_millis(),
        runtime.timezone_override(),
    );
    set_document_string_return_value(scope, &mut rv, &value);
}

fn document_referrer_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let referrer = runtime
        .child_browsing_context_referrer_for_document_handle(handle)
        .or_else(|| runtime.lightweight_popup_referrer_for_document_handle(handle))
        .unwrap_or("");
    set_document_string_return_value(scope, &mut rv, referrer);
}

fn document_domain_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let value = unsafe { &*runtime_ptr }.document_domain_value_for_document_handle(handle);
    set_document_string_return_value(scope, &mut rv, &value);
}

fn document_domain_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = args.get(0).to_string(scope) else {
        rv.set_undefined();
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.set_document_domain_for_document_handle(handle, &value) {
        let updated_context_count =
            runtime.refresh_security_tokens_after_document_domain_mutation(scope, handle);
        tracing::debug!(
            ?handle,
            updated_context_count,
            "refreshed Window security tokens after document.domain mutation"
        );
        rv.set_undefined();
        return;
    }
    throw_document_domain_security_error(scope);
    rv.set_undefined();
}

fn document_current_script_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_null();
        return;
    };
    if detached_native_handle_for_runtime(scope, runtime_ptr, receiver).is_some() {
        rv.set_null();
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    if !node_is_document(runtime, handle) {
        rv.set_null();
        return;
    }
    let current_script = current_script_handle_for_document(runtime, handle);
    set_wrapped_handle_or_null(scope, &mut rv, runtime_ptr, current_script);
}

fn current_script_handle_for_document(
    runtime: &JsContextHost,
    document_handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(script) = runtime.current_inline_script_handle()
        && current_script_belongs_to_document(runtime, script, document_handle)
    {
        return current_script_is_visible_for_document(runtime, script, document_handle)
            .then_some(script);
    }
    if let Some(script) = runtime.child_current_script_handle_for_document(document_handle) {
        return current_script_is_visible_for_document(runtime, script, document_handle)
            .then_some(script);
    }
    runtime
        .current_script_handle()
        .filter(|script| current_script_is_visible_for_document(runtime, *script, document_handle))
}

fn current_script_is_visible_for_document(
    runtime: &JsContextHost,
    script: DomHandle,
    document_handle: DomHandle,
) -> bool {
    let dom = runtime.dom_host();
    current_script_belongs_to_document(runtime, script, document_handle)
        && (!dom.is_connected(script) || dom.containing_shadow_root(script).is_none())
}

fn current_script_belongs_to_document(
    runtime: &JsContextHost,
    script: DomHandle,
    document_handle: DomHandle,
) -> bool {
    let dom = runtime.dom_host();
    dom.node(script)
        .and_then(Node::owner_document)
        .is_some_and(|owner_document| owner_document == document_handle)
}

fn document_hidden_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if document_receiver_runtime_and_handle(scope, args.this()).is_none() {
        rv.set_undefined();
        return;
    }
    rv.set_bool(false);
}

fn document_visibility_state_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if document_receiver_runtime_and_handle(scope, args.this()).is_none() {
        rv.set_undefined();
        return;
    }
    set_document_string_return_value(scope, &mut rv, "visible");
}

fn document_prerendering_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if document_receiver_runtime_and_handle(scope, args.this()).is_none() {
        rv.set_undefined();
        return;
    }
    rv.set_bool(false);
}

fn document_scrolling_element_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let dom = runtime.dom_host().dom();
    let element = dom
        .node(handle)
        .and_then(Node::as_document)
        .and_then(|document| document.document_element_handle(dom, handle));
    set_document_node_return_value_for_receiver(scope, &mut rv, runtime_ptr, receiver, element);
}

fn document_forms_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    document_html_collection_getter(scope, args, rv, DocumentCollectionAccessorKind::Forms);
}

fn document_images_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    document_html_collection_getter(scope, args, rv, DocumentCollectionAccessorKind::Images);
}

fn document_scripts_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    document_html_collection_getter(scope, args, rv, DocumentCollectionAccessorKind::Scripts);
}

fn document_links_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    document_html_collection_getter(scope, args, rv, DocumentCollectionAccessorKind::Links);
}

fn document_anchors_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    document_html_collection_getter(scope, args, rv, DocumentCollectionAccessorKind::Anchors);
}

fn document_embeds_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    document_html_collection_getter(scope, args, rv, DocumentCollectionAccessorKind::Embeds);
}

fn document_plugins_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    document_html_collection_getter(scope, args, rv, DocumentCollectionAccessorKind::Plugins);
}

fn document_applets_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    document_html_collection_getter(scope, args, rv, DocumentCollectionAccessorKind::Applets);
}

#[derive(Clone, Copy)]
enum DocumentCollectionAccessorKind {
    Forms,
    Images,
    Scripts,
    Links,
    Anchors,
    Embeds,
    Plugins,
    Applets,
}

fn document_html_collection_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    kind: DocumentCollectionAccessorKind,
) {
    let receiver = args.this();
    let Some((runtime_ptr, handle)) = document_receiver_runtime_and_handle(scope, receiver) else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !is_html_document(runtime, handle) {
        rv.set_undefined();
        return;
    }
    if detached_native_handle_for_runtime(scope, runtime_ptr, receiver).is_some() {
        match detached_document_collection_for_kind(scope, receiver, kind) {
            Some(collection) => rv.set(collection.into()),
            None => rv.set_null(),
        }
        return;
    }
    let (query_kind, query, tag_name_html_document) = match kind {
        DocumentCollectionAccessorKind::Forms => (LiveCollectionQueryKind::Forms, None, None),
        DocumentCollectionAccessorKind::Images => (LiveCollectionQueryKind::Images, None, None),
        DocumentCollectionAccessorKind::Scripts => (LiveCollectionQueryKind::Scripts, None, None),
        DocumentCollectionAccessorKind::Links => (LiveCollectionQueryKind::Links, None, None),
        DocumentCollectionAccessorKind::Anchors => (LiveCollectionQueryKind::Anchors, None, None),
        DocumentCollectionAccessorKind::Embeds | DocumentCollectionAccessorKind::Plugins => (
            LiveCollectionQueryKind::TagName,
            Some("embed".to_owned()),
            Some(true),
        ),
        DocumentCollectionAccessorKind::Applets => (
            LiveCollectionQueryKind::TagName,
            Some("__moli-never-match__".to_owned()),
            Some(true),
        ),
    };
    let descriptor = LiveCollectionDescriptor {
        collection_kind: CollectionKind::HtmlCollection,
        query_kind,
        root: handle,
        query,
        include_root: true,
        tag_name_html_document,
        resolution_cache: Default::default(),
    };
    let collection = collections::build_live_collection_wrapper(scope, runtime_ptr, descriptor);
    rv.set(collection.into());
}

fn detached_document_collection_for_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    kind: DocumentCollectionAccessorKind,
) -> Option<v8::Local<'s, v8::Object>> {
    match kind {
        DocumentCollectionAccessorKind::Forms => detached_document_forms_value(scope, document),
        DocumentCollectionAccessorKind::Images => detached_document_images_value(scope, document),
        DocumentCollectionAccessorKind::Scripts => detached_document_scripts_value(scope, document),
        DocumentCollectionAccessorKind::Links => detached_document_links_value(scope, document),
        DocumentCollectionAccessorKind::Anchors => detached_document_anchors_value(scope, document),
        DocumentCollectionAccessorKind::Embeds | DocumentCollectionAccessorKind::Plugins => {
            detached_document_embeds_value(scope, document)
        }
        DocumentCollectionAccessorKind::Applets => detached_document_applets_value(scope, document),
    }
}

fn document_default_view_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_document(runtime, handle) {
        rv.set_null();
        return;
    }
    if let Some(window) = get_private_value(scope, args.this(), DOCUMENT_ASSOCIATED_WINDOW_SLOT)
        && !window.is_null_or_undefined()
    {
        rv.set(window);
        return;
    }
    if runtime.dom_host().document_handle() != handle {
        rv.set_null();
        return;
    }
    rv.set(scope.get_current_context().global(scope).into());
}

pub(in crate::native_bridge) fn set_document_associated_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    window: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        document,
        DOCUMENT_ASSOCIATED_WINDOW_SLOT,
        window.into(),
    );
}

pub(crate) fn install_document_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    if interface_name == "Document" {
        DocumentMetadataPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        DocumentStructurePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        DocumentFocusPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        DocumentViewPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        DocumentStatePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        DocumentCollectionAccessorsPrototypeDeclaration::initialize_prototype_template(
            scope, prototype,
        );
        DocumentCollectionQueryPrototypeDeclaration::initialize_prototype_template(
            scope, prototype,
        );
    }
    if matches!(interface_name, "DocumentFragment" | "ShadowRoot") {
        DocumentFragmentCollectionQueryPrototypeDeclaration::initialize_prototype_template(
            scope, prototype,
        );
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document")]
struct DocumentGetElementByIdTemplateDeclaration {
    #[webapi(method = "getElementById", length = 1, callback = node_get_element_by_id_callback)]
    get_element_by_id: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Document", enumerable)]
struct DocumentPrototypeMethodsDeclaration {
    #[webapi(method = "createElement", length = 1, callback = node_create_element_callback)]
    create_element: (),
    #[webapi(method = "createAttribute", length = 1, callback = node_create_attribute_callback)]
    create_attribute: (),
    #[webapi(
        method = "createAttributeNS",
        length = 2,
        callback = node_create_attribute_ns_callback
    )]
    create_attribute_ns: (),
    #[webapi(
        method = "createElementNS",
        length = 2,
        callback = node_create_element_ns_callback
    )]
    create_element_ns: (),
    #[webapi(method = "createTextNode", length = 1, callback = node_create_text_node_callback)]
    create_text_node: (),
    #[webapi(method = "createComment", length = 1, callback = node_create_comment_callback)]
    create_comment: (),
    #[webapi(
        method = "createDocumentFragment",
        length = 0,
        callback = node_create_document_fragment_callback
    )]
    create_document_fragment: (),
    #[webapi(
        method = "createProcessingInstruction",
        length = 2,
        callback = node_create_processing_instruction_callback
    )]
    create_processing_instruction: (),
    #[webapi(
        method = "createCDATASection",
        length = 1,
        callback = node_create_cdata_section_callback
    )]
    create_cdata_section: (),
    #[webapi(method = "importNode", length = 2, callback = node_import_node_callback)]
    import_node: (),
    #[webapi(method = "adoptNode", length = 1, callback = node_adopt_node_callback)]
    adopt_node: (),
    #[webapi(method, length = 0, callback = node_document_write_callback)]
    write: (),
    #[webapi(method, length = 0, callback = node_document_writeln_callback)]
    writeln: (),
    #[webapi(method, length = 0, callback = node_document_open_callback)]
    open: (),
    #[webapi(method, length = 0, callback = node_document_close_callback)]
    close: (),
    #[webapi(method = "execCommand", length = 1, callback = node_document_exec_command_callback)]
    exec_command: (),
    #[webapi(
        method = "queryCommandEnabled",
        length = 1,
        callback = node_document_query_command_enabled_callback
    )]
    query_command_enabled: (),
    #[webapi(
        method = "queryCommandIndeterm",
        length = 1,
        callback = node_document_query_command_indeterm_callback
    )]
    query_command_indeterm: (),
    #[webapi(
        method = "queryCommandState",
        length = 1,
        callback = node_document_query_command_state_callback
    )]
    query_command_state: (),
    #[webapi(
        method = "queryCommandSupported",
        length = 1,
        callback = node_document_query_command_supported_callback
    )]
    query_command_supported: (),
    #[webapi(
        method = "queryCommandValue",
        length = 1,
        callback = node_document_query_command_value_callback
    )]
    query_command_value: (),
    #[webapi(
        method = "elementFromPoint",
        length = 2,
        callback = node_document_element_from_point_callback
    )]
    element_from_point: (),
    #[webapi(
        method = "elementsFromPoint",
        length = 2,
        callback = node_document_elements_from_point_callback
    )]
    elements_from_point: (),
    #[webapi(
        method = "caretPositionFromPoint",
        length = 2,
        callback = node_document_caret_position_from_point_callback
    )]
    caret_position_from_point: (),
    #[webapi(
        method = "createNodeIterator",
        length = 1,
        callback = node_create_node_iterator_callback
    )]
    create_node_iterator: (),
    #[webapi(
        method = "createTreeWalker",
        length = 1,
        callback = node_create_tree_walker_callback
    )]
    create_tree_walker: (),
    #[webapi(
        method = "createNSResolver",
        length = 1,
        callback = node_document_create_ns_resolver_callback
    )]
    create_ns_resolver: (),
    #[webapi(method, length = 5, callback = node_document_evaluate_callback)]
    evaluate: (),
    #[webapi(
        method = "hasStorageAccess",
        length = 0,
        callback = node_document_has_storage_access_callback
    )]
    has_storage_access: (),
    #[webapi(
        method = "requestStorageAccess",
        length = 0,
        callback = node_document_request_storage_access_callback
    )]
    request_storage_access: (),
}

pub(crate) fn install_document_prototype_methods<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    DocumentGetElementByIdTemplateDeclaration::initialize_prototype_template(scope, prototype);
    DocumentPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::native_bridge) fn node_document_has_storage_access_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((resolver, runtime_ptr)) =
        document_storage_access_resolver(scope, &args, &mut rv, "hasStorageAccess")
    else {
        return;
    };
    // SAFETY: document_storage_access_resolver returns a live JsContextHost
    // pointer from this isolate's native bridge wrapper.
    let has_access = document_storage_access_has_secure_context(scope, unsafe { &*runtime_ptr });
    let _ = resolver.resolve(scope, v8::Boolean::new(scope, has_access).into());
}

pub(in crate::native_bridge) fn node_document_request_storage_access_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((resolver, runtime_ptr)) =
        document_storage_access_resolver(scope, &args, &mut rv, "requestStorageAccess")
    else {
        return;
    };
    // SAFETY: document_storage_access_resolver returns a live JsContextHost
    // pointer from this isolate's native bridge wrapper.
    if !document_storage_access_has_secure_context(scope, unsafe { &*runtime_ptr }) {
        reject_document_storage_access_dom_exception(
            scope,
            resolver,
            "document.requestStorageAccess() is not allowed in an insecure context.",
            "NotAllowedError",
        );
        return;
    }
    if args.length() > 0 {
        crate::context_bootstrap::request_storage_access_with_types(scope, &args, resolver);
        return;
    }
    let _ = resolver.resolve(scope, v8::undefined(scope).into());
}

fn document_storage_access_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    method: &'static str,
) -> Option<(v8::Local<'s, v8::PromiseResolver>, *mut JsContextHost)> {
    let resolver = v8::PromiseResolver::new(scope)?;
    rv.set(resolver.get_promise(scope).into());
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        reject_document_storage_access_receiver(scope, resolver, method);
        return None;
    };
    // SAFETY: node_runtime_and_handle_from_object only returns a non-null
    // JsContextHost pointer stored on a native bridge wrapper for this isolate.
    if !node_is_document(unsafe { &*runtime_ptr }, handle) {
        reject_document_storage_access_receiver(scope, resolver, method);
        return None;
    }
    // SAFETY: node_runtime_and_handle_from_object_or_detached only returns a
    // non-null JsContextHost pointer from this isolate's native bridge state.
    if !unsafe { &*runtime_ptr }.dom_host().is_connected(handle) {
        reject_document_storage_access_dom_exception(
            scope,
            resolver,
            "Storage access is not available for a non-fully-active document.",
            "InvalidStateError",
        );
        return None;
    }
    Some((resolver, runtime_ptr))
}

fn document_storage_access_has_secure_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
) -> bool {
    let secure_context_url = runtime
        .current_runtime_window_execution_context_identity(scope)
        .and_then(|identity| {
            runtime.secure_context_url_for_window_execution_context_identity(identity)
        });
    secure_context_url
        .as_ref()
        .is_some_and(moli_url::is_potentially_trustworthy_url)
}

fn reject_document_storage_access_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    method: &'static str,
) {
    let message = format!("Failed to execute '{method}' on 'Document': Illegal invocation.");
    let error = v8_string(scope, &message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, error);
}

fn reject_document_storage_access_dom_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
    name: &str,
) {
    let error = dom_exception_value(scope, message, name);
    let _ = resolver.reject(scope, error);
}
pub(super) fn normalize_namespace(namespace: Option<String>) -> Option<String> {
    match namespace.as_deref() {
        None | Some("") => None,
        Some(_) => namespace,
    }
}

fn detached_method_forward<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    helper_name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let mut forwarded = Vec::with_capacity(args.length() as usize + 1);
    forwarded.push(args.this().into());
    for index in 0..args.length() {
        forwarded.push(args.get(index));
    }
    call_global_bridge_method(scope, helper_name, &forwarded)
}

pub(super) fn is_html_document(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_document)
        .is_some_and(|document| document.is_html_document())
}
