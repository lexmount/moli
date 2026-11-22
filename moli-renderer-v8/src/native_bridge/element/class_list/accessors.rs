use super::*;

use crate::util::{throw_type_error, v8str};

pub(in crate::native_bridge) fn blocking_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    match runtime
        .native_bridge_mut()
        .wrap_blocking_list(scope, runtime_ptr, handle)
    {
        Some(blocking) => rv.set(blocking.into()),
        None => rv.set_null(),
    }
}

pub(in crate::native_bridge) fn blocking_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(target) = args.this().get(scope, v8str(scope, "blocking").into()) else {
        return;
    };
    let Ok(target) = v8::Local::<v8::Object>::try_from(target) else {
        throw_type_error(scope, "The blocking getter did not return an object.");
        return;
    };
    // [PutForwards=value] passes the original value to the property setter.
    let _ = target.set(scope, v8str(scope, "value").into(), args.get(0));
}

pub(in crate::native_bridge) fn html_rel_list_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    match runtime
        .native_bridge_mut()
        .wrap_rel_list(scope, runtime_ptr, handle)
    {
        Some(rel_list) => rv.set(rel_list.into()),
        None => rv.set_null(),
    }
}

fn set_html_rel_list_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, value, owner, "relList") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "rel", &value);
}

pub(in crate::native_bridge) fn html_rel_list_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(interface) =
        super::super::reflection::ElementReflectionInterface::from_callback_data(scope, args.data())
    {
        set_html_rel_list_for_receiver(scope, args.this(), args.get(0), interface.name());
    }
    rv.set_undefined();
}
