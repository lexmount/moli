use moli_dom::html_microsyntax::parse_non_negative_integer;

use crate::util::v8_string;
use crate::webidl;

use super::super::{
    element_attribute, html_element_getter_receiver, html_element_setter_receiver,
    property_usv_string_value, resolve_url_like_attribute, set_reflected_attribute,
};

fn video_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(
    *mut crate::native_bridge::JsContextHost,
    crate::document_runtime::DomHandle,
)> {
    html_element_getter_receiver(scope, receiver, "HTMLVideoElement", member, "video")
}

fn video_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(
    *mut crate::native_bridge::JsContextHost,
    crate::document_runtime::DomHandle,
)> {
    html_element_setter_receiver(scope, receiver, "HTMLVideoElement", member, "video")
}

pub(in crate::native_bridge) fn media_poster_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = video_getter_receiver(scope, args.this(), "poster") else {
        rv.set_null();
        return;
    };
    let value = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, "poster");
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn media_poster_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = video_setter_receiver(scope, args.this(), "poster") else {
        rv.set_undefined();
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), "HTMLVideoElement", "poster")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "poster", &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_video_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if video_getter_receiver(scope, args.this(), "videoWidth").is_none() {
        rv.set_uint32(0);
        return;
    }
    rv.set_uint32(0);
}

pub(in crate::native_bridge) fn media_video_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if video_getter_receiver(scope, args.this(), "videoHeight").is_none() {
        rv.set_uint32(0);
        return;
    }
    rv.set_uint32(0);
}

pub(in crate::native_bridge) fn media_width_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_unsigned_long_attribute_getter(scope, args.this(), rv, "width");
}

pub(in crate::native_bridge) fn media_width_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_unsigned_long_attribute_setter(scope, args.this(), "width", args.get(0), "width");
    rv.set_undefined();
}

pub(in crate::native_bridge) fn media_height_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_unsigned_long_attribute_getter(scope, args.this(), rv, "height");
}

pub(in crate::native_bridge) fn media_height_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    media_unsigned_long_attribute_setter(scope, args.this(), "height", args.get(0), "height");
    rv.set_undefined();
}

fn media_unsigned_long_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    attribute: &'static str,
) {
    let Some((runtime_ptr, handle)) = video_getter_receiver(scope, object, attribute) else {
        rv.set_uint32(0);
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, attribute)
        .and_then(|value| parse_non_negative_integer(&value))
        .filter(|value| *value <= i32::MAX as u32)
        .unwrap_or(0);
    rv.set_uint32(value);
}

fn media_unsigned_long_attribute_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    attribute: &str,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
) {
    let Some((runtime_ptr, handle)) = video_setter_receiver(scope, object, member) else {
        return;
    };
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member("HTMLVideoElement", member),
    ) {
        Ok(value) if value.0 <= i32::MAX as u32 => value.0,
        Ok(_) => 0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value.to_string());
}
