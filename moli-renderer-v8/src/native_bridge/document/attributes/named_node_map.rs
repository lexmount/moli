use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

mod cache;
mod handlers;
mod helpers;
mod methods;

pub(crate) use cache::live_named_node_map_wrapper;
pub(in crate::native_bridge::document) use handlers::named_node_map_length_getter_callback;
pub(in crate::native_bridge::document) use methods::{
    named_node_map_get_named_item_method_callback,
    named_node_map_get_named_item_ns_method_callback, named_node_map_item_method_callback,
    named_node_map_remove_named_item_method_callback,
    named_node_map_remove_named_item_ns_method_callback,
    named_node_map_set_named_item_method_callback,
    named_node_map_set_named_item_ns_method_callback,
};
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::NamedNodeMap, enumerable)]
struct NamedNodeMapPrototypeDeclaration {
    #[webapi(accessor_property, getter = named_node_map_length_getter_callback)]
    length: (),

    #[webapi(method, length = 1, callback = named_node_map_item_method_callback)]
    item: (),

    #[webapi(
        method,
        length = 1,
        callback = named_node_map_get_named_item_method_callback
    )]
    get_named_item: (),

    #[webapi(
        method,
        length = 1,
        callback = named_node_map_set_named_item_method_callback
    )]
    set_named_item: (),

    #[webapi(
        method,
        length = 1,
        callback = named_node_map_remove_named_item_method_callback
    )]
    remove_named_item: (),

    #[webapi(
        method = "getNamedItemNS",
        length = 2,
        callback = named_node_map_get_named_item_ns_method_callback
    )]
    get_named_item_ns: (),

    #[webapi(
        method = "setNamedItemNS",
        length = 1,
        callback = named_node_map_set_named_item_ns_method_callback
    )]
    set_named_item_ns: (),

    #[webapi(
        method = "removeNamedItemNS",
        length = 2,
        callback = named_node_map_remove_named_item_ns_method_callback
    )]
    remove_named_item_ns: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

pub(crate) fn install_named_node_map_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "NamedNodeMap" {
        NamedNodeMapPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

pub(in crate::native_bridge) fn build_named_node_map_wrapper_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let _ = template.set_internal_field_count(1);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(handlers::named_node_map_indexed_getter)
            .setter(handlers::named_node_map_indexed_setter)
            .query(handlers::named_node_map_indexed_query)
            .deleter(handlers::named_node_map_indexed_deleter)
            .enumerator(handlers::named_node_map_indexed_enumerator)
            .definer(handlers::named_node_map_indexed_definer)
            .descriptor(handlers::named_node_map_indexed_descriptor),
    );
    template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(handlers::named_node_map_named_getter)
            .setter(handlers::named_node_map_named_setter)
            .query(handlers::named_node_map_named_query)
            .deleter(handlers::named_node_map_named_deleter)
            .enumerator(handlers::named_node_map_named_enumerator)
            .definer(handlers::named_node_map_named_definer)
            .descriptor(handlers::named_node_map_named_descriptor),
    );
    template
}
