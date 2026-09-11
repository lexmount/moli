use crate::util::{get_private_object, throw_type_error};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{
    DataPropertyDescriptorDeclaration, WebApiFunctionTemplate, WebApiObject,
};

const DOM_STRING_LIST_VALUES_SLOT: &str = "moli.IndexedDb.DOMStringListValues";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DOMStringList, enumerable)]
struct DomStringListPrototypeDeclaration {
    #[webapi(accessor_property, getter = dom_string_list_length_getter)]
    length: (),
    #[webapi(method, length = 1, callback = dom_string_list_contains_callback)]
    contains: (),
    #[webapi(method, length = 1, callback = dom_string_list_item_callback)]
    item: (),
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DOMStringList, require_prototype)]
struct DomStringListObjectDeclaration<'s> {
    #[webapi(slot = DOM_STRING_LIST_VALUES_SLOT)]
    values: v8::Local<'s, v8::Array>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMStringList.item")]
struct DomStringListItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMStringList.contains")]
struct DomStringListContainsArgs {
    #[webidl(required, name = "string")]
    expected: String,
}

pub(in crate::context_bootstrap::indexed_db) fn new_idb_dom_string_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &[String],
) -> v8::Local<'s, v8::Object> {
    let values =
        crate::util::serialize_v8_array(scope, values).unwrap_or_else(|| v8::Array::new(scope, 0));
    let template = v8::ObjectTemplate::new(scope);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(dom_string_list_indexed_getter)
            .setter(dom_string_list_indexed_setter)
            .deleter(dom_string_list_indexed_deleter)
            .enumerator(dom_string_list_indexed_enumerator)
            .definer(dom_string_list_indexed_definer)
            .descriptor(dom_string_list_indexed_descriptor),
    );
    let object = template
        .new_instance(scope)
        .expect("IDB DOMStringList object template should instantiate");
    DomStringListObjectDeclaration::new(values)
        .bind_into(scope, object)
        .expect("IDB DOMStringList declaration should bind");
    object
}

pub(in crate::context_bootstrap::indexed_db) fn install_dom_string_list_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface_name: &str,
) {
    if interface_name == "DOMStringList" {
        DomStringListPrototypeDeclaration::initialize_prototype_template(scope, prototype);
    }
}

pub(in crate::context_bootstrap::indexed_db) fn idb_dom_string_list_backing_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_object(scope, object, DOM_STRING_LIST_VALUES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn require_dom_string_list_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    idb_dom_string_list_backing_values(scope, object).or_else(|| {
        throw_type_error(
            scope,
            &format!("Failed to execute '{member}' on 'DOMStringList': Illegal invocation."),
        );
        None
    })
}

fn dom_string_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(values) = require_dom_string_list_values(scope, args.this(), "get length") else {
        return;
    };
    rv.set_uint32(values.length());
}

fn dom_string_list_contains_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(values) = require_dom_string_list_values(scope, args.this(), "contains") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<DomStringListContainsArgs>(scope, &args) else {
        return;
    };
    for index in 0..values.length() {
        let Some(value) = values.get_index(scope, index) else {
            continue;
        };
        if value
            .to_string(scope)
            .is_some_and(|value| value.to_rust_string_lossy(scope) == parsed.expected)
        {
            rv.set_bool(true);
            return;
        }
    }
    rv.set_bool(false);
}

fn dom_string_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(values) = require_dom_string_list_values(scope, args.this(), "item") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<DomStringListItemArgs>(scope, &args) else {
        return;
    };
    match values.get_index(scope, parsed.index) {
        Some(value) if value.is_null_or_undefined() => rv.set(v8::null(scope).into()),
        Some(value) => rv.set(value),
        None => rv.set(v8::null(scope).into()),
    }
}

fn dom_string_list_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = idb_dom_string_list_backing_values(scope, args.holder())
        .filter(|values| index < values.length())
        .and_then(|values| values.get_index(scope, index))
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn dom_string_list_indexed_setter(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _value: v8::Local<'_, v8::Value>,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn dom_string_list_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(values) = idb_dom_string_list_backing_values(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if index >= values.length() {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn dom_string_list_indexed_definer(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn dom_string_list_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let length = idb_dom_string_list_backing_values(scope, args.holder())
        .map(|values| values.length())
        .unwrap_or(0);
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn dom_string_list_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = idb_dom_string_list_backing_values(scope, args.holder())
        .filter(|values| index < values.length())
        .and_then(|values| values.get_index(scope, index))
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, false, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}
