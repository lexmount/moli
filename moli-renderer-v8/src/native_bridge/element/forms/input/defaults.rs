use super::super::*;

pub(in crate::native_bridge) fn input_default_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "value", rv);
}

pub(in crate::native_bridge) fn input_default_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "value",
        args.get(0),
        "HTMLInputElement",
        "defaultValue",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn input_default_checked_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    input_default_checked_getter_from_object(scope, args.this(), &mut rv);
}

fn input_default_checked_getter_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(has_reflected_attribute(
        unsafe { &*runtime_ptr },
        handle,
        "checked",
    ));
}

pub(in crate::native_bridge) fn input_default_checked_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let checked = args.get(0).boolean_value(scope);
    set_reflected_boolean_attribute(scope, runtime_ptr, handle, "checked", checked);
    rv.set_undefined();
}
