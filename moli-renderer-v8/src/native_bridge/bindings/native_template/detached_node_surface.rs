use crate::native_bridge::document;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "NativeBridgeDetachedNodeSurface", enumerable)]
struct NativeBridgeDetachedNodeSurfaceDeclaration {
    #[webapi(
        method = "__detachedNodeType",
        callback = document::bridge_detached_node_type_callback
    )]
    detached_node_type: (),

    #[webapi(
        method = "__detachedNodeName",
        callback = document::bridge_detached_node_name_callback
    )]
    detached_node_name: (),

    #[webapi(
        method = "__detachedNodeValue",
        callback = document::bridge_detached_node_value_getter_callback
    )]
    detached_node_value: (),

    #[webapi(
        method = "__setDetachedNodeValue",
        callback = document::bridge_detached_node_value_setter_callback
    )]
    set_detached_node_value: (),

    #[webapi(
        method = "__detachedDoctypeName",
        callback = document::bridge_detached_doctype_name_callback
    )]
    detached_doctype_name: (),

    #[webapi(
        method = "__detachedDoctypePublicId",
        callback = document::bridge_detached_doctype_public_id_callback
    )]
    detached_doctype_public_id: (),

    #[webapi(
        method = "__detachedDoctypeSystemId",
        callback = document::bridge_detached_doctype_system_id_callback
    )]
    detached_doctype_system_id: (),

    #[webapi(
        method = "__detachedCharacterData",
        callback = document::bridge_detached_character_data_getter_callback
    )]
    detached_character_data: (),

    #[webapi(
        method = "__setDetachedCharacterData",
        callback = document::bridge_detached_character_data_setter_callback
    )]
    set_detached_character_data: (),

    #[webapi(
        method = "__detachedProcessingInstructionTarget",
        callback = document::bridge_detached_processing_instruction_target_callback
    )]
    detached_processing_instruction_target: (),

    #[webapi(
        method = "__detachedElementNamespaceURI",
        callback = document::bridge_detached_element_namespace_uri_callback
    )]
    detached_element_namespace_uri: (),

    #[webapi(
        method = "__detachedElementPrefix",
        callback = document::bridge_detached_element_prefix_callback
    )]
    detached_element_prefix: (),

    #[webapi(
        method = "__detachedElementLocalName",
        callback = document::bridge_detached_element_local_name_callback
    )]
    detached_element_local_name: (),

    #[webapi(
        method = "__detachedElementTagName",
        callback = document::bridge_detached_element_tag_name_callback
    )]
    detached_element_tag_name: (),

    #[webapi(
        method = "__detachedParentNode",
        callback = document::bridge_detached_parent_node_callback
    )]
    detached_parent_node: (),

    #[webapi(
        method = "__detachedParentElement",
        callback = document::bridge_detached_parent_element_callback
    )]
    detached_parent_element: (),

    #[webapi(
        method = "__detachedOwnerDocument",
        callback = document::bridge_detached_owner_document_callback
    )]
    detached_owner_document: (),

    #[webapi(
        method = "__detachedChildNodes",
        callback = document::bridge_detached_child_nodes_callback
    )]
    detached_child_nodes: (),

    #[webapi(
        method = "__detachedFirstChild",
        callback = document::bridge_detached_first_child_callback
    )]
    detached_first_child: (),

    #[webapi(
        method = "__detachedLastChild",
        callback = document::bridge_detached_last_child_callback
    )]
    detached_last_child: (),

    #[webapi(
        method = "__detachedPreviousSibling",
        callback = document::bridge_detached_previous_sibling_callback
    )]
    detached_previous_sibling: (),

    #[webapi(
        method = "__detachedNextSibling",
        callback = document::bridge_detached_next_sibling_callback
    )]
    detached_next_sibling: (),

    #[webapi(
        method = "__detachedIsConnected",
        callback = document::bridge_detached_is_connected_callback
    )]
    detached_is_connected: (),

    #[webapi(
        method = "__detachedHasChildNodes",
        callback = document::bridge_detached_has_child_nodes_callback
    )]
    detached_has_child_nodes: (),

    #[webapi(
        method = "__detachedChildren",
        callback = document::bridge_detached_children_callback
    )]
    detached_children: (),

    #[webapi(
        method = "__detachedFirstElementChild",
        callback = document::bridge_detached_first_element_child_callback
    )]
    detached_first_element_child: (),

    #[webapi(
        method = "__detachedLastElementChild",
        callback = document::bridge_detached_last_element_child_callback
    )]
    detached_last_element_child: (),

    #[webapi(
        method = "__detachedChildElementCount",
        callback = document::bridge_detached_child_element_count_callback
    )]
    detached_child_element_count: (),

    #[webapi(
        method = "__detachedPreviousElementSibling",
        callback = document::bridge_detached_previous_element_sibling_callback
    )]
    detached_previous_element_sibling: (),

    #[webapi(
        method = "__detachedNextElementSibling",
        callback = document::bridge_detached_next_element_sibling_callback
    )]
    detached_next_element_sibling: (),

    #[webapi(
        method = "__detachedCloneNode",
        callback = document::bridge_detached_clone_node_callback
    )]
    detached_clone_node: (),

    #[webapi(
        method = "__detachedTextContent",
        callback = document::bridge_detached_text_content_callback
    )]
    detached_text_content: (),
}

pub(super) fn install_detached_node_surface<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
) {
    NativeBridgeDetachedNodeSurfaceDeclaration::initialize_prototype_template(scope, template);
}
