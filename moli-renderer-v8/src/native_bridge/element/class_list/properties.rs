use super::{
    identity::class_list_runtime_handle_and_kind_from_object,
    tokens::{class_list_tokens, token_list_attribute_name},
};

pub(super) fn class_list_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let length = class_list_tokens(unsafe { &*runtime_ptr }, handle, kind).len() as i32;
    rv.set(v8::Integer::new(scope, length).into());
}

pub(super) fn class_list_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let units = unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute_ns_utf16_units(handle, None, token_list_attribute_name(kind))
        .unwrap_or_default();
    let Some(value) = crate::util::v8_string_from_utf16_units(scope, &units) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(super) fn class_list_value_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle, kind)) =
        class_list_runtime_handle_and_kind_from_object(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(units) = super::super::reflection::property_dom_string_utf16_value(
        scope,
        args.get(0),
        "DOMTokenList",
        "value",
    ) else {
        rv.set_undefined();
        return;
    };
    super::super::reflection::set_reflected_attribute_utf16(
        scope,
        runtime_ptr,
        handle,
        token_list_attribute_name(kind),
        units,
    );
    rv.set_undefined();
}
