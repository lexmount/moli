use crate::{
    util::{get_private_object, serialize_v8_array, throw_type_error},
    webidl,
};
use moli_webapi_declare::{
    DataPropertyDescriptorDeclaration, WebApiFunctionTemplate, WebApiObject,
};

const DOM_RECT_LIST_VALUES_SLOT: &str = "__moliDomRectListValues";

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMRectList", enumerable)]
struct DomRectListPrototypeDeclaration {
    #[webapi(accessor_property, getter = dom_rect_list_length_getter)]
    length: (),

    #[webapi(method, length = 1, callback = dom_rect_list_item_callback)]
    item: (),
}

#[derive(WebApiObject)]
#[webapi(
    interface = "DOMRectList",
    require_prototype,
    fallback_to_string_tag = "DOMRectList"
)]
struct DomRectListObjectDeclaration<'scope> {
    #[webapi(slot = DOM_RECT_LIST_VALUES_SLOT)]
    values: v8::Local<'scope, v8::Array>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMRectList.item")]
struct DomRectListItemArgs {
    #[webidl(required)]
    index: u32,
}

pub(crate) fn build_dom_rect_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rects: &[v8::Local<'s, v8::Object>],
) -> v8::Local<'s, v8::Object> {
    let values = serialize_v8_array(scope, rects).unwrap_or_else(|| v8::Array::new(scope, 0));
    let template = v8::ObjectTemplate::new(scope);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(dom_rect_list_indexed_getter)
            .setter(dom_rect_list_indexed_setter)
            .query(dom_rect_list_indexed_query)
            .deleter(dom_rect_list_indexed_deleter)
            .enumerator(dom_rect_list_indexed_enumerator)
            .definer(dom_rect_list_indexed_definer)
            .descriptor(dom_rect_list_indexed_descriptor),
    );
    let object = template
        .new_instance(scope)
        .expect("DOMRectList object template should instantiate");
    DomRectListObjectDeclaration::new(values)
        .bind_into(scope, object)
        .expect("DOMRectList declaration should bind");
    object
}

pub(crate) fn is_dom_rect_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    dom_rect_list_values(scope, object).is_some()
}

pub(in crate::context_bootstrap) fn install_dom_rect_list_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "DOMRectList" {
        DomRectListPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

fn dom_rect_list_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_object(scope, object, DOM_RECT_LIST_VALUES_SLOT)
        .and_then(|values| v8::Local::<v8::Array>::try_from(values).ok())
}

fn require_dom_rect_list_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    dom_rect_list_values(scope, object).or_else(|| {
        throw_type_error(
            scope,
            &format!("Failed to execute '{member}' on 'DOMRectList': Illegal invocation."),
        );
        None
    })
}

fn dom_rect_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(values) = require_dom_rect_list_values(scope, args.this(), "get length") else {
        return;
    };
    rv.set_uint32(values.length());
}

fn dom_rect_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(values) = require_dom_rect_list_values(scope, args.this(), "item") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<DomRectListItemArgs>(scope, &args) else {
        return;
    };
    match values.get_index(scope, parsed.index) {
        Some(value) if !value.is_null_or_undefined() => rv.set(value),
        _ => rv.set(v8::null(scope).into()),
    }
}

fn dom_rect_list_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = dom_rect_list_values(scope, args.holder())
        .filter(|values| index < values.length())
        .and_then(|values| values.get_index(scope, index))
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn dom_rect_list_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Some(values) = dom_rect_list_values(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if index >= values.length() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

fn dom_rect_list_indexed_setter(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _value: v8::Local<'_, v8::Value>,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn dom_rect_list_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(values) = dom_rect_list_values(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if index >= values.length() {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn dom_rect_list_indexed_definer(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    rv.set_bool(true);
    v8::Intercepted::kYes
}

fn dom_rect_list_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let length = dom_rect_list_values(scope, args.holder())
        .map(|values| values.length())
        .unwrap_or(0);
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn dom_rect_list_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = dom_rect_list_values(scope, args.holder())
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
