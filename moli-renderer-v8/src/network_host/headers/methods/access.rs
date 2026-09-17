use super::super::store::{get_header_prop, headers_list, normalized_header_name_or_throw};
use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Headers.get")]
struct HeadersNameArgs {
    #[webidl(required, converter = "byte_string")]
    name: String,
}

pub(in crate::network_host::headers) fn headers_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(this) = require_headers_receiver(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<HeadersNameArgs>(scope, &args) else {
        rv.set_null();
        return;
    };
    let name = parsed.name;
    if normalized_header_name_or_throw(scope, &name).is_none() {
        return;
    }
    match get_header_prop(scope, this, &name) {
        Some(val) => rv.set(val),
        None => rv.set_null(),
    }
}

pub(in crate::network_host::headers) fn headers_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_headers_receiver(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<HeadersNameArgs>(scope, &args) else {
        rv.set_bool(false);
        return;
    };
    let name = parsed.name;
    if normalized_header_name_or_throw(scope, &name).is_none() {
        return;
    }
    rv.set_bool(get_header_prop(scope, this, &name).is_some());
}

pub(in crate::network_host::headers) fn headers_get_set_cookie_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_headers_receiver(scope, args.this()) else {
        return;
    };
    let values = headers_list(scope, this)
        .into_iter()
        .filter_map(|(name, value)| (name == "set-cookie").then_some(value))
        .filter_map(|value| v8_string(scope, &value).map(Into::into))
        .collect::<Vec<v8::Local<'_, v8::Value>>>();
    rv.set(v8::Array::new_with_elements(scope, &values).into());
}
