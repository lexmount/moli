use crate::{
    util::{v8_string_from_utf16_units, v8_value_to_dom_string_u16},
    webidl,
};

use super::*;

pub(super) fn character_data_utf16_units(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<Vec<u16>> {
    runtime.character_data_utf16_units(handle)
}

pub(in crate::native_bridge) fn require_argument_count(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
    interface: &str,
    operation: &str,
    required: i32,
) -> bool {
    if args.length() >= required {
        return true;
    }
    let plural = if required == 1 { "" } else { "s" };
    let message = format!(
        "Failed to execute '{operation}' on '{interface}': {required} argument{plural} required, but only {} present.",
        args.length()
    );
    throw_type_error(scope, &message);
    false
}

pub(in crate::native_bridge) fn utf16_index_value_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    len: usize,
    context: webidl::Context,
) -> Option<usize> {
    let start = match webidl::convert::<webidl::UnsignedLong>(scope, value, context) {
        Ok(value) => value.0 as usize,
        Err(error) => {
            throw_type_error(scope, &error.to_string());
            return None;
        }
    };
    if start > len {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "Index or size is negative or greater than the allowed amount",
        );
        return None;
    }
    Some(start)
}

pub(in crate::native_bridge) fn utf16_count_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Option<usize> {
    match webidl::convert::<webidl::UnsignedLong>(scope, value, context) {
        Ok(value) => Some(value.0 as usize),
        Err(error) => {
            throw_type_error(scope, &error.to_string());
            None
        }
    }
}

pub(in crate::native_bridge) fn dom_string_value_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Option<String> {
    match webidl::convert::<webidl::DomString>(scope, value, context) {
        Ok(value) => Some(value.0),
        Err(error) => {
            throw_type_error(scope, &error.to_string());
            None
        }
    }
}

pub(super) fn dom_string_utf16_value_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    _context: webidl::Context,
    treat_null_as_empty_string: bool,
) -> Option<Vec<u16>> {
    v8_value_to_dom_string_u16(scope, value, treat_null_as_empty_string)
        .map(|value| value.into_vec())
}

pub(super) fn set_utf16_return_value(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    units: &[u16],
) {
    if let Some(value) = v8_string_from_utf16_units(scope, units) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}
