use super::*;
use crate::native_bridge::element::{
    node_query_selector_all_callback, node_query_selector_callback,
};
use moli_webapi_declare::WebApiObject;

mod document;
mod live_attribute_bridge;
mod node;

use document::install_detached_document_methods;
use live_attribute_bridge::install_live_attribute_bridge_methods;
use node::{install_detached_node_methods, install_detached_parent_node_move_before};

#[derive(WebApiObject)]
#[webapi(unbranded, interface = "ParentNode")]
struct DocumentFragmentParentNodeQueryMethodsDeclaration {
    #[webapi(method, callback = node_query_selector_callback)]
    query_selector: (),

    #[webapi(method, callback = node_query_selector_all_callback)]
    query_selector_all: (),
}

#[derive(WebApiObject)]
#[webapi(unbranded, interface = "DocumentFragment")]
struct DocumentFragmentGetElementByIdDeclaration {
    #[webapi(method, length = 1, callback = document_fragment_get_element_by_id_callback)]
    get_element_by_id: (),
}

pub(in crate::native_bridge) fn install_detached_bridge_methods(scope: &mut v8::PinScope<'_, '_>) {
    let Some(bridge) = global_bridge_object(scope) else {
        return;
    };
    ensure_detached_bridge_prototypes(scope);
    install_live_attribute_bridge_methods(scope, bridge);

    let Some(element_prototype) = bridge_prototype_object(scope, "__detachedElementPrototype")
    else {
        return;
    };
    let Some(html_document_prototype) =
        bridge_prototype_object(scope, "__detachedHTMLDocumentPrototype")
    else {
        return;
    };
    let Some(xml_document_prototype) =
        bridge_prototype_object(scope, "__detachedXMLDocumentPrototype")
    else {
        return;
    };
    if let Some(document_fragment_prototype) =
        global_constructor_prototype(scope, "DocumentFragment")
    {
        DocumentFragmentGetElementByIdDeclaration::new()
            .initialize(scope, document_fragment_prototype)
            .expect("DocumentFragment getElementById declaration should initialize");
        // ParentNode mixin members exposed on DocumentFragment per spec.
        // Use live-aware callbacks here; detached-only callbacks are for bridge
        // forwarders and parse their selector at argument index 1.
        DocumentFragmentParentNodeQueryMethodsDeclaration::new()
            .initialize(scope, document_fragment_prototype)
            .expect("DocumentFragment ParentNode query method declaration should initialize");
    }
    if let Some(shadow_root_prototype) =
        bridge_prototype_object(scope, "__detachedShadowRootPrototype")
    {
        DocumentFragmentGetElementByIdDeclaration::new()
            .initialize(scope, shadow_root_prototype)
            .expect("detached ShadowRoot getElementById declaration should initialize");
        DocumentFragmentParentNodeQueryMethodsDeclaration::new()
            .initialize(scope, shadow_root_prototype)
            .expect("detached ShadowRoot ParentNode query method declaration should initialize");
    }
    let plain_document_prototype = bridge_prototype_object(scope, "__detachedDocumentPrototype");
    let detached_node_prototypes: Vec<v8::Local<'_, v8::Object>> = [
        bridge_prototype_object(scope, "__detachedDocumentTypePrototype"),
        bridge_prototype_object(scope, "__detachedDocumentFragmentPrototype"),
        bridge_prototype_object(scope, "__detachedShadowRootPrototype"),
        bridge_prototype_object(scope, "__detachedTextPrototype"),
        bridge_prototype_object(scope, "__detachedCommentPrototype"),
        bridge_prototype_object(scope, "__detachedProcessingInstructionPrototype"),
        bridge_prototype_object(scope, "__detachedCDATASectionPrototype"),
        Some(element_prototype),
        Some(html_document_prototype),
        Some(xml_document_prototype),
        plain_document_prototype,
    ]
    .into_iter()
    .flatten()
    .collect();
    let detached_parent_node_prototypes: Vec<v8::Local<'_, v8::Object>> = [
        bridge_prototype_object(scope, "__detachedDocumentFragmentPrototype"),
        bridge_prototype_object(scope, "__detachedShadowRootPrototype"),
        Some(element_prototype),
        Some(html_document_prototype),
        Some(xml_document_prototype),
        plain_document_prototype,
    ]
    .into_iter()
    .flatten()
    .collect();

    install_detached_node_methods(scope, &detached_node_prototypes);
    for prototype in detached_parent_node_prototypes {
        install_detached_parent_node_move_before(scope, prototype);
    }
    install_detached_document_methods(
        scope,
        html_document_prototype,
        xml_document_prototype,
        plain_document_prototype,
    );
}

fn document_fragment_get_element_by_id_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if crate::native_bridge::node_runtime_and_handle_from_object(scope, args.this()).is_ok() {
        node_get_element_by_id_callback(scope, args, rv);
    } else {
        detached_get_element_by_id_method_callback(scope, args, rv);
    }
}
