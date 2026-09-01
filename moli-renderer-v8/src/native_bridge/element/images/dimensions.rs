use crate::document_runtime::DomHandle;
use crate::webidl;

use super::super::super::{JsContextHost, node::node_runtime_and_handle_from_object_or_detached};
use super::super::{element_attribute, parse_non_negative_dimension, set_reflected_attribute};

pub(in crate::native_bridge) fn image_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_uint32(image_width_value(scope, args.this()));
}

pub(in crate::native_bridge) fn image_width_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_image_unsigned_long_attribute_on_object(scope, args.this(), "width", args.get(0), "width");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn image_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_uint32(image_height_value(scope, args.this()));
}

pub(in crate::native_bridge) fn image_height_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_image_unsigned_long_attribute_on_object(
        scope,
        args.this(),
        "height",
        args.get(0),
        "height",
    );
    rv.set_undefined();
}

fn image_width_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> u32 {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return 0;
    };
    let runtime = unsafe { &*runtime_ptr };
    element_attribute(runtime, handle, "width")
        .map(|value| parse_non_negative_dimension(Some(value)))
        .filter(|value| *value > 0)
        .or_else(|| image_natural_dimensions(runtime, handle).map(|(width, _)| width))
        .unwrap_or(0)
}

fn image_height_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> u32 {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return 0;
    };
    let runtime = unsafe { &*runtime_ptr };
    element_attribute(runtime, handle, "height")
        .map(|value| parse_non_negative_dimension(Some(value)))
        .filter(|value| *value > 0)
        .or_else(|| image_natural_dimensions(runtime, handle).map(|(_, height)| height))
        .unwrap_or(0)
}

fn set_image_unsigned_long_attribute_on_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
) {
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member("HTMLImageElement", member),
    ) {
        Ok(value) if value.0 <= i32::MAX as u32 => value.0,
        Ok(_) => 0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value.to_string());
}

pub(crate) fn image_natural_dimensions(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<(u32, u32)> {
    runtime.image_resource_natural_dimensions(handle)
}

pub(in crate::native_bridge) fn image_natural_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_uint32(image_natural_width_value(scope, args.this()));
}

pub(in crate::native_bridge) fn image_natural_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_uint32(image_natural_height_value(scope, args.this()));
}

fn image_natural_width_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> u32 {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return 0;
    };
    image_natural_dimensions(unsafe { &*runtime_ptr }, handle)
        .map(|(width, _)| width)
        .unwrap_or(0)
}

fn image_natural_height_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> u32 {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, object)
    else {
        return 0;
    };
    image_natural_dimensions(unsafe { &*runtime_ptr }, handle)
        .map(|(_, height)| height)
        .unwrap_or(0)
}
