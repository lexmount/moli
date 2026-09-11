use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(Default, WebApiObject)]
#[webapi(plain, enumerable)]
struct NodeFilterConstantsObjectDeclaration {
    #[webapi(constant = "FILTER_ACCEPT", value = 1u32)]
    _filter_accept: (),
    #[webapi(constant = "FILTER_REJECT", value = 2u32)]
    _filter_reject: (),
    #[webapi(constant = "FILTER_SKIP", value = 3u32)]
    _filter_skip: (),
    #[webapi(constant = "SHOW_ALL", value = 0xFFFF_FFFFu32)]
    _show_all: (),
    #[webapi(constant = "SHOW_ELEMENT", value = 0x1u32)]
    _show_element: (),
    #[webapi(constant = "SHOW_ATTRIBUTE", value = 0x2u32)]
    _show_attribute: (),
    #[webapi(constant = "SHOW_TEXT", value = 0x4u32)]
    _show_text: (),
    #[webapi(constant = "SHOW_CDATA_SECTION", value = 0x8u32)]
    _show_cdata_section: (),
    #[webapi(constant = "SHOW_ENTITY_REFERENCE", value = 0x10u32)]
    _show_entity_reference: (),
    #[webapi(constant = "SHOW_ENTITY", value = 0x20u32)]
    _show_entity: (),
    #[webapi(constant = "SHOW_PROCESSING_INSTRUCTION", value = 0x40u32)]
    _show_processing_instruction: (),
    #[webapi(constant = "SHOW_COMMENT", value = 0x80u32)]
    _show_comment: (),
    #[webapi(constant = "SHOW_DOCUMENT", value = 0x100u32)]
    _show_document: (),
    #[webapi(constant = "SHOW_DOCUMENT_TYPE", value = 0x200u32)]
    _show_document_type: (),
    #[webapi(constant = "SHOW_DOCUMENT_FRAGMENT", value = 0x400u32)]
    _show_document_fragment: (),
    #[webapi(constant = "SHOW_NOTATION", value = 0x800u32)]
    _show_notation: (),
}

#[derive(WebApiObject)]
#[webapi(fragment, prototype = "Window")]
struct NodeFilterGlobalDeclaration<'scope> {
    #[webapi(data_property = "NodeFilter")]
    node_filter: v8::Local<'scope, v8::Function>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Node, enumerable)]
struct NodeConstantsDeclaration {
    #[webapi(constant = "ELEMENT_NODE", value = 1u32)]
    _element_node: (),
    #[webapi(constant = "ATTRIBUTE_NODE", value = 2u32)]
    _attribute_node: (),
    #[webapi(constant = "TEXT_NODE", value = 3u32)]
    _text_node: (),
    #[webapi(constant = "CDATA_SECTION_NODE", value = 4u32)]
    _cdata_section_node: (),
    #[webapi(constant = "ENTITY_REFERENCE_NODE", value = 5u32)]
    _entity_reference_node: (),
    #[webapi(constant = "ENTITY_NODE", value = 6u32)]
    _entity_node: (),
    #[webapi(constant = "PROCESSING_INSTRUCTION_NODE", value = 7u32)]
    _processing_instruction_node: (),
    #[webapi(constant = "COMMENT_NODE", value = 8u32)]
    _comment_node: (),
    #[webapi(constant = "DOCUMENT_NODE", value = 9u32)]
    _document_node: (),
    #[webapi(constant = "DOCUMENT_TYPE_NODE", value = 10u32)]
    _document_type_node: (),
    #[webapi(constant = "DOCUMENT_FRAGMENT_NODE", value = 11u32)]
    _document_fragment_node: (),
    #[webapi(constant = "NOTATION_NODE", value = 12u32)]
    _notation_node: (),
    #[webapi(constant = "DOCUMENT_POSITION_DISCONNECTED", value = 0x01u32)]
    _document_position_disconnected: (),
    #[webapi(constant = "DOCUMENT_POSITION_PRECEDING", value = 0x02u32)]
    _document_position_preceding: (),
    #[webapi(constant = "DOCUMENT_POSITION_FOLLOWING", value = 0x04u32)]
    _document_position_following: (),
    #[webapi(constant = "DOCUMENT_POSITION_CONTAINS", value = 0x08u32)]
    _document_position_contains: (),
    #[webapi(constant = "DOCUMENT_POSITION_CONTAINED_BY", value = 0x10u32)]
    _document_position_contained_by: (),
    #[webapi(
        constant = "DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC",
        value = 0x20u32
    )]
    _document_position_implementation_specific: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLTrackElement, enumerable)]
struct HtmlTrackElementConstantsDeclaration {
    #[webapi(constant = "NONE", value = 0u32)]
    _none: (),
    #[webapi(constant = "LOADING", value = 1u32)]
    _loading: (),
    #[webapi(constant = "LOADED", value = 2u32)]
    _loaded: (),
    #[webapi(constant = "ERROR", value = 3u32)]
    _error: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLMediaElement, enumerable)]
struct MediaElementPrototypeConstantsDeclaration {
    #[webapi(constant = "NETWORK_EMPTY", value = 0u32)]
    _network_empty: (),
    #[webapi(constant = "NETWORK_IDLE", value = 1u32)]
    _network_idle: (),
    #[webapi(constant = "NETWORK_LOADING", value = 2u32)]
    _network_loading: (),
    #[webapi(constant = "NETWORK_NO_SOURCE", value = 3u32)]
    _network_no_source: (),
    #[webapi(constant = "HAVE_NOTHING", value = 0u32)]
    _have_nothing: (),
    #[webapi(constant = "HAVE_METADATA", value = 1u32)]
    _have_metadata: (),
    #[webapi(constant = "HAVE_CURRENT_DATA", value = 2u32)]
    _have_current_data: (),
    #[webapi(constant = "HAVE_FUTURE_DATA", value = 3u32)]
    _have_future_data: (),
    #[webapi(constant = "HAVE_ENOUGH_DATA", value = 4u32)]
    _have_enough_data: (),
}

pub(in crate::context_bootstrap) fn install_constructor_constant_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Node" => {
            NodeConstantsDeclaration::initialize_template(scope, template);
            NodeConstantsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "HTMLTrackElement" => {
            HtmlTrackElementConstantsDeclaration::initialize_template(scope, template);
            HtmlTrackElementConstantsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "HTMLMediaElement" | "HTMLAudioElement" | "HTMLVideoElement" => {
            MediaElementPrototypeConstantsDeclaration::initialize_template(scope, template);
            MediaElementPrototypeConstantsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "WebGLRenderingContext" | "WebGL2RenderingContext" => {
            for (name, value) in super::super::canvas::WEBGL_CONSTANTS {
                let value = v8::Number::new(scope, *value as f64);
                template.set_with_attr(
                    v8str(scope, name).into(),
                    value.into(),
                    v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                );
                prototype.set_with_attr(
                    v8str(scope, name).into(),
                    value.into(),
                    v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                );
            }
            if interface_name == "WebGL2RenderingContext" {
                for (name, value) in super::super::canvas::WEBGL2_CONSTANTS {
                    let value = v8::Number::new(scope, *value as f64);
                    template.set_with_attr(
                        v8str(scope, name).into(),
                        value.into(),
                        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                    );
                    prototype.set_with_attr(
                        v8str(scope, name).into(),
                        value.into(),
                        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                    );
                }
            }
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn install_node_filter_constants<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) {
    let template = v8::FunctionTemplate::builder(illegal_constructor_callback)
        .length(0)
        .build(scope);
    template.remove_prototype();
    let node_filter = template
        .get_function(scope)
        .expect("NodeFilter callback interface object should materialize");
    node_filter.set_name(v8str(scope, "NodeFilter"));
    NodeFilterConstantsObjectDeclaration::default()
        .initialize(scope, node_filter.into())
        .expect("NodeFilter constants declaration should initialize");
    NodeFilterGlobalDeclaration::new(node_filter)
        .initialize(scope, global)
        .expect("NodeFilter global declaration should initialize");
}
