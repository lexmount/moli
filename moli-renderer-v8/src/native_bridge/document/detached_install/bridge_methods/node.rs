use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(Default, WebApiObject)]
#[webapi(unbranded, interface = "Node")]
struct DetachedNodeMethodsDeclaration {
    #[webapi(method, callback = detached_append_child_method_callback)]
    append_child: (),

    #[webapi(method, callback = detached_insert_before_method_callback)]
    insert_before: (),

    #[webapi(method, callback = detached_remove_child_method_callback)]
    remove_child: (),

    #[webapi(method, callback = detached_remove_method_callback)]
    remove: (),

    #[webapi(method, callback = detached_replace_child_method_callback)]
    replace_child: (),

    #[webapi(method, callback = detached_has_child_nodes_method_callback)]
    has_child_nodes: (),

    #[webapi(method, callback = detached_get_root_node_method_callback)]
    get_root_node: (),

    #[webapi(
        method = "lookupNamespaceURI",
        callback = detached_lookup_namespace_uri_method_callback
    )]
    lookup_namespace_uri: (),

    #[webapi(method, callback = detached_contains_method_callback)]
    contains: (),

    #[webapi(method, callback = detached_is_same_node_method_callback)]
    is_same_node: (),

    #[webapi(method, callback = detached_is_equal_node_method_callback)]
    is_equal_node: (),

    #[webapi(method, callback = detached_compare_document_position_method_callback)]
    compare_document_position: (),

    #[webapi(method, callback = detached_clone_node_method_callback)]
    clone_node: (),

    #[webapi(method, callback = detached_append_method_callback)]
    append: (),

    #[webapi(method, callback = detached_prepend_method_callback)]
    prepend: (),

    #[webapi(method, callback = detached_replace_children_method_callback)]
    replace_children: (),

    #[webapi(method, callback = detached_before_method_callback)]
    before: (),

    #[webapi(method, callback = detached_after_method_callback)]
    after: (),

    #[webapi(method, callback = detached_replace_with_method_callback)]
    replace_with: (),

    #[webapi(method, callback = detached_normalize_method_callback)]
    normalize: (),
}

pub(super) fn install_detached_parent_node_move_before<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) {
    #[derive(Default, WebApiObject)]
    #[webapi(unbranded, interface = "ParentNode")]
    struct DetachedParentNodeMoveBeforeDeclaration {
        #[webapi(method, length = 2, callback = detached_move_before_method_callback)]
        move_before: (),
    }

    DetachedParentNodeMoveBeforeDeclaration::default()
        .initialize(scope, prototype)
        .expect("detached ParentNode moveBefore declaration should initialize");
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
