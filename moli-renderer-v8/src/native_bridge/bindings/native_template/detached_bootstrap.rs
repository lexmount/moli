use crate::native_bridge::document;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "NativeBridgeDetachedBootstrap", enumerable)]
struct NativeBridgeDetachedBootstrapDeclaration {
    #[webapi(
        method = "__cloneNodeIntoDocument",
        callback = document::bridge_clone_node_into_document_callback
    )]
    clone_node_into_document: (),

    #[webapi(
        method = "__createDetachedDocument",
        callback = document::bridge_create_detached_document_callback
    )]
    create_detached_document: (),

    #[webapi(
        method = "__createDetachedDocumentFragment",
        callback = document::bridge_create_detached_document_fragment_callback
    )]
    create_detached_document_fragment: (),

    #[webapi(
        method = "__createDetachedText",
        callback = document::bridge_create_detached_text_callback
    )]
    create_detached_text: (),

    #[webapi(
        method = "__createDetachedComment",
        callback = document::bridge_create_detached_comment_callback
    )]
    create_detached_comment: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "NativeBridgeDetachedCreationHelpers", enumerable)]
struct NativeBridgeDetachedCreationHelpersDeclaration {
    #[webapi(
        method = "__createDetachedDocumentType",
        callback = document::bridge_create_detached_document_type_callback
    )]
    create_detached_document_type: (),

    #[webapi(
        method = "__createDetachedHTMLDocument",
        callback = document::bridge_create_detached_html_document_callback
    )]
    create_detached_html_document: (),

    #[webapi(
        method = "__createDetachedXmlDocument",
        callback = document::bridge_create_detached_xml_document_callback
    )]
    create_detached_xml_document: (),

    #[webapi(
        method = "__detachedCreateText",
        callback = document::bridge_detached_create_text_callback
    )]
    detached_create_text: (),

    #[webapi(
        method = "__detachedCreateComment",
        callback = document::bridge_detached_create_comment_callback
    )]
    detached_create_comment: (),

    #[webapi(
        method = "__detachedCreateDocumentFragment",
        callback = document::bridge_detached_create_document_fragment_callback
    )]
    detached_create_document_fragment: (),

    #[webapi(
        method = "__detachedCreateProcessingInstruction",
        callback = document::bridge_detached_create_processing_instruction_callback
    )]
    detached_create_processing_instruction: (),

    #[webapi(
        method = "__detachedCreateCDATASection",
        callback = document::bridge_detached_create_cdata_section_callback
    )]
    detached_create_cdata_section: (),

    #[webapi(
        method = "__adoptNodeIntoDocument",
        callback = document::bridge_adopt_node_into_document_callback
    )]
    adopt_node_into_document: (),
}

pub(super) fn install_detached_bootstrap<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
) {
    NativeBridgeDetachedBootstrapDeclaration::initialize_prototype_template(scope, template);
}

pub(super) fn install_detached_creation_helpers<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
) {
    NativeBridgeDetachedCreationHelpersDeclaration::initialize_prototype_template(scope, template);
}
