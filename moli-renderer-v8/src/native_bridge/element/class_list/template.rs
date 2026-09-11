use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DOMTokenList, enumerable)]
struct DomTokenListTemplateMethodsDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoEntries,
        enumerable
    )]
    entries: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoKeys,
        enumerable
    )]
    keys: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        enumerable
    )]
    values: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoForEach,
        enumerable
    )]
    for_each: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),

    #[webapi(method, length = 1, callback = methods::class_list_item_callback)]
    item: (),

    #[webapi(
        method,
        length = 1,
        callback = methods::class_list_contains_callback
    )]
    contains: (),

    #[webapi(method, length = 1, callback = methods::class_list_add_callback)]
    add: (),

    #[webapi(method, length = 1, callback = methods::class_list_remove_callback)]
    remove: (),

    #[webapi(method, length = 1, callback = methods::class_list_toggle_callback)]
    toggle: (),

    #[webapi(method, length = 2, callback = methods::class_list_replace_callback)]
    replace: (),

    #[webapi(method, length = 1, callback = methods::class_list_supports_callback)]
    supports: (),

    #[webapi(method, length = 0, callback = methods::class_list_to_string_callback)]
    to_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DOMTokenList)]
struct DomTokenListAttributeDescriptorsDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = properties::class_list_length_getter_callback
    )]
    length: (),

    #[webapi(
        accessor_property,
        enumerable,
        getter = properties::class_list_value_getter_callback,
        setter = properties::class_list_value_setter_callback
    )]
    value: (),
}

pub(in crate::native_bridge) fn build_dom_token_list_wrapper_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let _ = template.set_internal_field_count(1);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed::class_list_indexed_getter)
            .setter(indexed::class_list_indexed_setter)
            .query(indexed::class_list_indexed_query)
            .deleter(indexed::class_list_indexed_deleter)
            .enumerator(indexed::class_list_indexed_enumerator)
            .definer(indexed::class_list_indexed_definer)
            .descriptor(indexed::class_list_indexed_descriptor),
    );
    template
}

pub(crate) fn install_dom_token_list_prototype_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let proto = template.prototype_template(scope);
    DomTokenListTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
    DomTokenListAttributeDescriptorsDeclaration::initialize_prototype_template(scope, proto);
}
