use crate::native_bridge::document;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "NativeBridgeDetachedMutationAndAttributeHelpers", enumerable)]
struct NativeBridgeDetachedMutationAndAttributeHelpersDeclaration {
    #[webapi(
        method = "__detachedAppend",
        callback = document::bridge_detached_append_callback
    )]
    detached_append: (),

    #[webapi(
        method = "__detachedPrepend",
        callback = document::bridge_detached_prepend_callback
    )]
    detached_prepend: (),

    #[webapi(
        method = "__detachedReplaceChildren",
        callback = document::bridge_detached_replace_children_callback
    )]
    detached_replace_children: (),

    #[webapi(
        method = "__detachedBefore",
        callback = document::bridge_detached_before_callback
    )]
    detached_before: (),

    #[webapi(
        method = "__detachedAfter",
        callback = document::bridge_detached_after_callback
    )]
    detached_after: (),

    #[webapi(
        method = "__detachedReplaceWith",
        callback = document::bridge_detached_replace_with_callback
    )]
    detached_replace_with: (),

    #[webapi(
        method = "__detachedMoveBefore",
        callback = document::bridge_detached_move_before_callback
    )]
    detached_move_before: (),

    #[webapi(
        method = "__detachedRemoveChild",
        callback = document::bridge_detached_remove_child_callback
    )]
    detached_remove_child: (),

    #[webapi(
        method = "__detachedGetAttribute",
        callback = document::bridge_detached_get_attribute_callback
    )]
    detached_get_attribute: (),

    #[webapi(
        method = "__detachedGetAttributeNS",
        callback = document::bridge_detached_get_attribute_ns_callback
    )]
    detached_get_attribute_ns: (),

    #[webapi(
        method = "__detachedGetAttributeNames",
        callback = document::bridge_detached_get_attribute_names_callback
    )]
    detached_get_attribute_names: (),

    #[webapi(
        method = "__detachedHasAttribute",
        callback = document::bridge_detached_has_attribute_callback
    )]
    detached_has_attribute: (),

    #[webapi(
        method = "__detachedHasAttributeNS",
        callback = document::bridge_detached_has_attribute_ns_callback
    )]
    detached_has_attribute_ns: (),

    #[webapi(
        method = "__detachedSetAttribute",
        callback = document::bridge_detached_set_attribute_callback
    )]
    detached_set_attribute: (),

    #[webapi(
        method = "__detachedSetAttributeNS",
        callback = document::bridge_detached_set_attribute_ns_callback
    )]
    detached_set_attribute_ns: (),

    #[webapi(
        method = "__detachedRemoveAttribute",
        callback = document::bridge_detached_remove_attribute_callback
    )]
    detached_remove_attribute: (),

    #[webapi(
        method = "__detachedRemoveAttributeNS",
        callback = document::bridge_detached_remove_attribute_ns_callback
    )]
    detached_remove_attribute_ns: (),
}

pub(super) fn install_detached_mutation_and_attribute_helpers<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
    template: v8::Local<'s, v8::ObjectTemplate>,
) {
    NativeBridgeDetachedMutationAndAttributeHelpersDeclaration::initialize_prototype_template(
        scope, template,
    );
}
