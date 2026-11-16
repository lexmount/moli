use super::*;
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSNumericArray)]
struct NumericArrayDeclaration<'s> {
    #[webapi(slot = VALUES_SLOT)]
    values: v8::Local<'s, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSNumericArray, enumerable, receiver)]
struct NumericArrayPrototype {
    #[webapi(accessor_property, getter = length_getter)]
    length: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoEntries)]
    entries: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoKeys)]
    keys: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues)]
    values: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoForEach)]
    for_each: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues, symbol = "iterator")]
    iterator: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    NumericArrayPrototype::initialize_prototype_template(scope, template.prototype_template(scope));
    install_indexed(template.instance_template(scope));
}

fn install_indexed(template: v8::Local<'_, v8::ObjectTemplate>) {
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(getter)
            .setter(setter)
            .query(query)
            .deleter(deleter)
            .definer(definer)
            .descriptor(descriptor)
            .enumerator(enumerator),
    );
}

pub(super) fn new<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Object> {
    let template = v8::ObjectTemplate::new(scope);
    install_indexed(template);
    let object = template
        .new_instance(scope)
        .expect("CSSNumericArray instance");
    NumericArrayDeclaration::new(values)
        .bind_into(scope, object)
        .expect("CSSNumericArray should bind");
    object
}

fn length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(values) = children(scope, args.this()) {
        rv.set_uint32(values.length());
    }
}

fn getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = children(scope, args.holder())
        .filter(|a| index < a.length())
        .and_then(|a| a.get_index(scope, index))
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if children(scope, args.holder()).is_none_or(|a| index >= a.length()) {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

fn setter(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _value: v8::Local<'_, v8::Value>,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn definer(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if children(scope, args.holder()).is_none_or(|a| index >= a.length()) {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = children(scope, args.holder())
        .filter(|a| index < a.length())
        .and_then(|a| a.get_index(scope, index))
    else {
        return v8::Intercepted::kNo;
    };
    if let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, false, true).bind(scope) {
        rv.set(descriptor.into());
    }
    v8::Intercepted::kYes
}

fn enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let len = children(scope, args.holder()).map_or(0, |a| a.length());
    let keys = (0..len)
        .map(|i| v8::Integer::new_from_unsigned(scope, i).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}
