use super::super::super::{
    custom_elements,
    util::{call_global_bridge_method, v8str},
};
use super::super::node::{
    node_arg_handle, node_is_document, node_or_foreign_arg_handle_allow_detached,
    node_runtime_and_handle_from_args, node_runtime_and_handle_from_args_or_detached,
    node_runtime_and_handle_from_object,
};
use super::super::{
    JsContextHost, throw_dom_exception, validate_attribute_name, validate_element_name,
    validate_qualified_element_name_and_namespace, validate_qualified_name_and_namespace,
};
use super::{
    XHTML_NS, clone_js_node_like_into_document_object, is_html_document, is_valid_pi_target,
    new_attr_object, normalize_namespace,
};
use super::{
    detached_adopt_node_method_callback, detached_create_attribute_method_callback,
    detached_create_attribute_ns_method_callback, detached_create_comment_method_callback,
    detached_create_document_fragment_method_callback,
    detached_create_html_element_method_callback, detached_create_html_element_ns_method_callback,
    detached_create_processing_instruction_method_callback,
    detached_create_text_node_method_callback, detached_create_xml_element_method_callback,
    detached_create_xml_element_ns_method_callback, detached_import_node_method_callback,
};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createElement")]
struct DocumentCreateElementArgs {
    #[webidl(required)]
    local_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createElementNS")]
struct DocumentCreateElementNsArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    qualified_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createTextNode")]
struct DocumentCreateTextNodeArgs {
    #[webidl(required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createComment")]
struct DocumentCreateCommentArgs {
    #[webidl(required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createProcessingInstruction")]
struct DocumentCreateProcessingInstructionArgs {
    #[webidl(required)]
    target: String,
    #[webidl(required)]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createAttribute")]
struct DocumentCreateAttributeArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createAttributeNS")]
struct DocumentCreateAttributeNsArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    qualified_name: String,
}

mod attributes;
mod bridge_callbacks;
mod character_nodes;
mod elements;
mod helpers;
mod import_adopt;

pub(in crate::native_bridge) use attributes::{
    node_create_attribute_callback, node_create_attribute_ns_callback,
};
pub(in crate::native_bridge) use bridge_callbacks::{
    bridge_create_comment_callback, bridge_create_element_callback,
    bridge_create_element_ns_callback, bridge_create_processing_instruction_callback,
    bridge_create_text_node_callback,
};
pub(in crate::native_bridge) use character_nodes::{
    node_create_cdata_section_callback, node_create_comment_callback,
    node_create_document_fragment_callback, node_create_processing_instruction_callback,
    node_create_text_node_callback,
};
pub(in crate::native_bridge) use elements::{
    node_create_element_callback, node_create_element_ns_callback,
};
pub(in crate::native_bridge) use import_adopt::{
    node_adopt_node_callback, node_import_node_callback,
};

#[derive(Clone, Copy)]
enum DetachedDocumentReceiverKind {
    Html,
    Xml,
}

fn detached_document_receiver_kind(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<DetachedDocumentReceiverKind> {
    if node_runtime_and_handle_from_args(scope, args).is_ok() {
        return None;
    }
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, args)
    else {
        return None;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_document(runtime, handle) {
        return None;
    }
    Some(if is_html_document(runtime, handle) {
        DetachedDocumentReceiverKind::Html
    } else {
        DetachedDocumentReceiverKind::Xml
    })
}
