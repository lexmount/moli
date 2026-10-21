use super::*;
use crate::native_bridge::node::{
    node_compare_document_position_callback, node_contains_callback, node_is_equal_node_callback,
    node_is_same_node_callback,
};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "Node")]
struct DetachedNodeMethodsDeclaration {
    #[webapi(method, callback = detached_remove_method_callback)]
    remove: (),

    #[webapi(method, callback = detached_has_child_nodes_method_callback)]
    has_child_nodes: (),

    #[webapi(method, callback = detached_get_root_node_method_callback)]
    get_root_node: (),

    #[webapi(
        method = "lookupNamespaceURI",
        callback = detached_lookup_namespace_uri_method_callback
    )]
    lookup_namespace_uri: (),

    #[webapi(method, length = 1, callback = node_contains_callback, receiver = web_api_interfaces::Node::is_instance)]
    contains: (),

    #[webapi(method, length = 1, callback = node_is_same_node_callback, receiver = web_api_interfaces::Node::is_instance)]
    is_same_node: (),

    #[webapi(method, length = 1, callback = node_is_equal_node_callback, receiver = web_api_interfaces::Node::is_instance)]
    is_equal_node: (),

    #[webapi(method, length = 1, callback = node_compare_document_position_callback, receiver = web_api_interfaces::Node::is_instance)]
    compare_document_position: (),

    #[webapi(method, callback = detached_clone_node_method_callback)]
    clone_node: (),

    #[webapi(method, callback = detached_before_method_callback)]
    before: (),

    #[webapi(method, callback = detached_after_method_callback)]
    after: (),

    #[webapi(method, callback = detached_replace_with_method_callback)]
    replace_with: (),

    #[webapi(method, callback = detached_normalize_method_callback)]
    normalize: (),
}

pub(super) fn install_detached_node_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototypes: &[v8::Local<'s, v8::Object>],
) {
    for prototype in prototypes {
        DetachedNodeMethodsDeclaration::default()
            .initialize(scope, *prototype)
            .expect("detached Node method declaration should initialize");
    }
}
