use super::*;

mod document;
mod node;

use document::install_detached_document_methods;
use node::install_detached_node_methods;

pub(in crate::native_bridge) fn install_detached_bridge_methods(scope: &mut v8::PinScope<'_, '_>) {
    if global_bridge_object(scope).is_none() {
        return;
    }
    ensure_detached_bridge_prototypes(scope);

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
    let plain_document_prototype = bridge_prototype_object(scope, "__detachedDocumentPrototype");
    let detached_node_prototypes: Vec<v8::Local<'_, v8::Object>> = [
        bridge_prototype_object(scope, "__detachedDocumentTypePrototype"),
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
    install_detached_document_methods(
        scope,
        html_document_prototype,
        xml_document_prototype,
        plain_document_prototype,
    );
}
