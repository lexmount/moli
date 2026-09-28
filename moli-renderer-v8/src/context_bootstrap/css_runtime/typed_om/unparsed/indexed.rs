use super::*;
use moli_webapi_declare::{DataPropertyDescriptorDeclaration, define_array_data_property};

pub(super) fn install(template: v8::Local<'_, v8::ObjectTemplate>) {
    // V8 calls the setter only when the interceptor holder is the receiver.
    // Inherited writes use ordinary receiver semantics; Proxy forwarding can
    // reach the definer instead. Both paths share conversion and range checks.
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

fn getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = segments(scope, args.holder())
        .filter(|values| index < values.length())
        .and_then(|values| values.get_index(scope, index))
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
    if segments(scope, args.holder()).is_some_and(|values| index < values.length()) {
        rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}

fn deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if segments(scope, args.holder()).is_some_and(|values| index < values.length()) {
        rv.set_bool(false);
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}

fn definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if !descriptor.has_value() && !descriptor.has_writable() {
        rv.set_bool(false);
        return v8::Intercepted::kYes;
    }
    let value = if descriptor.has_value() {
        v8::Local::new(scope, descriptor.value())
    } else {
        v8::undefined(scope).into()
    };
    store(scope, index, value, args, rv)
}

fn setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    store(scope, index, value, args, rv)
}

fn store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(values) = segments(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    let value = match webidl::convert::<Segment<'s>>(
        scope,
        value,
        webidl::Context::member("CSSUnparsedValue", "indexed setter"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return v8::Intercepted::kYes;
        }
    };
    // Conversion can append another segment. Check the live length afterward.
    if index > values.length() {
        crate::util::throw_range_error(scope, "CSSUnparsedValue index is outside its range");
        return v8::Intercepted::kYes;
    }
    rv.set_bool(define_array_data_property(scope, values, index, value).is_some());
    v8::Intercepted::kYes
}

fn descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = segments(scope, args.holder())
        .filter(|values| index < values.length())
        .and_then(|values| values.get_index(scope, index))
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, true, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

fn enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let length = segments(scope, args.holder()).map_or(0, |values| values.length());
    let keys = (0..length)
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}
