use super::*;
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

const VALUES_SLOT: &str = "__lmTouchListValues";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TouchList)]
struct TouchListObjectDeclaration<'s> {
    #[webapi(slot = VALUES_SLOT)]
    values: v8::Local<'s, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TouchList, enumerable, receiver)]
struct TouchListPrototypeDeclaration {
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues, symbol = "iterator")]
    iterator: (),
    #[webapi(accessor_property, getter = length_getter)]
    length: (),
    #[webapi(method, length = 1, callback = item)]
    item: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TouchList.item")]
struct ItemArgs {
    #[webidl(required)]
    index: u32,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    TouchListPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
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

pub(super) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    touches: &[v8::Local<'s, v8::Object>],
) -> v8::Local<'s, v8::Object> {
    let values = crate::util::serialize_v8_array(scope, touches).expect("TouchList values");
    let template = v8::ObjectTemplate::new(scope);
    install_indexed(template);
    let object = template.new_instance(scope).expect("TouchList instance");
    TouchListObjectDeclaration::new(values)
        .bind_into(scope, object)
        .expect("TouchList declaration should bind");
    object
}

fn values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let target = web_api_object_target(scope, object)?;
    v8::Local::try_from(get_private_value(scope, target, VALUES_SLOT)?).ok()
}

fn length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let values = values(scope, args.this()).expect("native TouchList receiver");
    rv.set_uint32(values.length());
}

fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ItemArgs>(scope, &args) else {
        return;
    };
    let values = values(scope, args.this()).expect("native TouchList receiver");
    if parsed.index < values.length() {
        rv.set(
            values
                .get_index(scope, parsed.index)
                .expect("TouchList entry"),
        );
    } else {
        rv.set(v8::null(scope).into());
    }
}

fn getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(values) = values(scope, args.holder()).filter(|a| index < a.length()) else {
        return v8::Intercepted::kNo;
    };
    rv.set(values.get_index(scope, index).expect("TouchList entry"));
    v8::Intercepted::kYes
}

fn query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if values(scope, args.holder()).is_none_or(|a| index >= a.length()) {
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
    // Read-only indexed properties reject definitions even outside their range.
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(values(scope, args.holder()).is_none_or(|a| index >= a.length()));
    v8::Intercepted::kYes
}

fn descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(values) = values(scope, args.holder()).filter(|a| index < a.length()) else {
        return v8::Intercepted::kNo;
    };
    let value = values.get_index(scope, index).expect("TouchList entry");
    let descriptor = DataPropertyDescriptorDeclaration::new(value, false, true)
        .bind(scope)
        .expect("TouchList indexed descriptor");
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

fn enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let length = values(scope, args.holder()).map_or(0, |a| a.length());
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}
