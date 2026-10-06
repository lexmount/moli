use super::{
    identity::class_list_runtime_handle_and_kind_from_object, tokens::class_list_tokens, *,
};
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

pub(super) fn class_list_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Some(token) = class_list_tokens(unsafe { &*runtime_ptr }, handle, kind)
        .get(index as usize)
        .cloned()
    else {
        return v8::Intercepted::kNo;
    };
    let Some(token) = v8_string_from_utf16_units(scope, &token) else {
        return v8::Intercepted::kNo;
    };
    rv.set(token.into());
    v8::Intercepted::kYes
}

pub(super) fn class_list_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if class_list_tokens(unsafe { &*runtime_ptr }, handle, kind).len() <= index as usize {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(super) fn class_list_indexed_setter(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _value: v8::Local<'_, v8::Value>,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(super) fn class_list_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if class_list_tokens(unsafe { &*runtime_ptr }, handle, kind).len() <= index as usize {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(super) fn class_list_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Some(token) = class_list_tokens(unsafe { &*runtime_ptr }, handle, kind)
        .get(index as usize)
        .cloned()
    else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = v8_string_from_utf16_units(scope, &token) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) =
        DataPropertyDescriptorDeclaration::new(value.into(), false, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(super) fn class_list_indexed_definer(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(super) fn class_list_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys = (0..class_list_tokens(unsafe { &*runtime_ptr }, handle, kind).len())
        .map(|index| v8::Integer::new(scope, index as i32).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}
