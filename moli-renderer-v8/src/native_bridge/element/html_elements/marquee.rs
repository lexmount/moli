use style::attr::{parse_integer, parse_unsigned_integer};

use crate::{native_bridge::throw_dom_exception, webidl};

use super::super::{
    html_element_getter_receiver, html_element_setter_receiver, reflected_attribute,
    set_reflected_attribute,
};

const DEFAULT_LOOP: i32 = -1;
const DEFAULT_SCROLL_AMOUNT: u32 = 6;
const DEFAULT_SCROLL_DELAY: u32 = 85;

pub(in crate::native_bridge::element) fn marquee_loop_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, args.this(), "HTMLMarqueeElement", "loop", "marquee")
    else {
        return;
    };
    let value = reflected_attribute(unsafe { &*runtime_ptr }, handle, "loop")
        .and_then(|value| parse_integer(value.chars()).ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_LOOP);
    rv.set_int32(value);
}

pub(in crate::native_bridge::element) fn marquee_loop_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, args.this(), "HTMLMarqueeElement", "loop", "marquee")
    else {
        return;
    };
    let value = match webidl::convert::<webidl::Long>(
        scope,
        args.get(0),
        webidl::Context::member("HTMLMarqueeElement", "loop"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if value <= 0 && value != DEFAULT_LOOP {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "The provided value is neither positive nor -1.",
        );
        return;
    }
    set_reflected_attribute(scope, runtime_ptr, handle, "loop", &value.to_string());
    rv.set_undefined();
}

fn marquee_unsigned_long_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
    attribute: &'static str,
    default_value: u32,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, receiver, "HTMLMarqueeElement", member, "marquee")
    else {
        return;
    };
    let value = reflected_attribute(unsafe { &*runtime_ptr }, handle, attribute)
        .and_then(|value| parse_unsigned_integer(value.chars()).ok())
        .filter(|value| *value <= i32::MAX as u32)
        .unwrap_or(default_value);
    rv.set_uint32(value);
}

fn marquee_unsigned_long_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
    attribute: &'static str,
    default_value: u32,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, receiver, "HTMLMarqueeElement", member, "marquee")
    else {
        return;
    };
    let value = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        value,
        webidl::Context::member("HTMLMarqueeElement", member),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let value = if value <= i32::MAX as u32 {
        value
    } else {
        default_value
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value.to_string());
    rv.set_undefined();
}

pub(in crate::native_bridge::element) fn marquee_scroll_amount_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    marquee_unsigned_long_getter(
        scope,
        args.this(),
        "scrollAmount",
        "scrollamount",
        DEFAULT_SCROLL_AMOUNT,
        rv,
    );
}

pub(in crate::native_bridge::element) fn marquee_scroll_amount_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    marquee_unsigned_long_setter(
        scope,
        args.this(),
        args.get(0),
        "scrollAmount",
        "scrollamount",
        DEFAULT_SCROLL_AMOUNT,
        rv,
    );
}

pub(in crate::native_bridge::element) fn marquee_scroll_delay_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    marquee_unsigned_long_getter(
        scope,
        args.this(),
        "scrollDelay",
        "scrolldelay",
        DEFAULT_SCROLL_DELAY,
        rv,
    );
}

pub(in crate::native_bridge::element) fn marquee_scroll_delay_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    marquee_unsigned_long_setter(
        scope,
        args.this(),
        args.get(0),
        "scrollDelay",
        "scrolldelay",
        DEFAULT_SCROLL_DELAY,
        rv,
    );
}
