//! Native indexed SourceBufferList. The backing array is private; indexed writes
//! follow Web IDL's readonly indexed getter rules, including unsupported indices.

use super::{LISTENERS, handler_getter, handler_setter, target};
use crate::{
    context_bootstrap::shared,
    util::{get_private_object, v8str},
    web_api_interfaces,
};
use moli_webapi_declare::{
    DataPropertyDescriptorDeclaration, WebApiFunctionTemplate, WebApiObject,
};

const OWNER: &str = "__moliSourceBufferListOwner";
const VALUES: &str = "__moliSourceBufferListValues";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SourceBufferList, require_prototype)]
struct ListObject<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = VALUES)]
    values: v8::Local<'s, v8::Array>,
    #[webapi(slot = shared::SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS)]
    event_target: (),
    #[webapi(slot = shared::SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SourceBufferList, receiver, enumerable)]
struct ListPrototype {
    #[webapi(accessor_property, getter = length)]
    length: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "addsourcebuffer"))]
    onaddsourcebuffer: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "removesourcebuffer"))]
    onremovesourcebuffer: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues, symbol = "iterator")]
    iterator: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ListHandler<'s> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    reflect_set: v8::Local<'s, v8::Function>,
    #[webapi(method, length = 4, callback = set, data = self.reflect_set)]
    set: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    ListPrototype::initialize_prototype_template(scope, template.prototype_template(scope));
}

pub(crate) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    values: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Object> {
    let template = v8::ObjectTemplate::new(scope);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed_getter)
            .query(indexed_query)
            .deleter(indexed_deleter)
            .definer(indexed_definer)
            .enumerator(indexed_enumerator)
            .descriptor(indexed_descriptor),
    );
    let object = template
        .new_instance(scope)
        .expect("SourceBufferList should allocate");
    ListObject::new(owner, values)
        .bind_into(scope, object)
        .expect("SourceBufferList should bind");
    // Interceptors expose their holder but not the original [[Set]] receiver.
    // A registered native Proxy retains brand identity while providing it.
    let handler = ListHandler::new(
        moli_webapi_declare::web_api_reflect_set(scope).expect("Reflect.set was captured"),
    )
    .bind(scope)
    .expect("SourceBufferList handler should bind");
    let proxy =
        v8::Proxy::new(scope, object, handler).expect("SourceBufferList Proxy should allocate");
    moli_webapi_declare::register_web_api_proxy(scope, proxy)
        .expect("SourceBufferList Proxy retains native identity");
    proxy.into()
}

fn values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let object = target(scope, receiver);
    get_private_object(scope, object, VALUES)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("SourceBufferList retains its backing array")
}

fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Value>> {
    let values = values(scope, receiver);
    (index < values.length())
        .then(|| values.get_index(scope, index))
        .flatten()
}

fn length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(values(scope, args.this()).length());
}

fn indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = item(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if index >= values(scope, args.holder()).length() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = v8::Local::<v8::Object>::try_from(args.get(0)).expect("native Proxy target");
    let key = args.get(1);
    let index = key
        .is_string()
        .then(|| key.to_rust_string_lossy(scope))
        .and_then(|key| {
            key.parse::<u32>()
                .ok()
                .filter(|index| *index != u32::MAX && index.to_string() == key)
        });
    let target = if let Some(index) = index {
        if index < values(scope, object).length() {
            rv.set_bool(false);
            return;
        }
        // Ordinary [[Set]] begins at the prototype for an unsupported index.
        // Its eventual own creation uses this Proxy's [[DefineOwnProperty]],
        // rather than V8's interceptor Set fast path which bypasses definer.
        object
            .get_prototype(scope)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .unwrap_or_else(|| crate::util::new_null_prototype_object(scope))
    } else {
        object
    };
    let reflect_set =
        v8::Local::<v8::Function>::try_from(args.data()).expect("captured Reflect.set");
    if let Some(result) = reflect_set.call(
        scope,
        v8::undefined(scope).into(),
        &[target.into(), key, args.get(2), args.get(3)],
    ) {
        rv.set(result);
    }
}

fn indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if index >= values(scope, args.holder()).length() {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn indexed_definer(
    _scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if index == u32::MAX {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let length = values(scope, args.holder()).length();
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = item(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    let descriptor = DataPropertyDescriptorDeclaration::new(value, false, true)
        .bind(scope)
        .expect("SourceBufferList indexed descriptor should allocate");
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}
