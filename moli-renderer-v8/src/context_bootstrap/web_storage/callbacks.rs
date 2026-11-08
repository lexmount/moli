use super::helpers::{storage_clear, storage_put_utf16, storage_remove_utf16, with_storage_result};
use crate::util::{v8_string_from_utf16_units, v8_value_to_dom_string_u16};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Storage.key")]
struct StorageKeyArgs {
    #[webidl(
        required,
        missing_message = "Failed to execute 'key' on 'Storage': 1 argument required, but only 0 present."
    )]
    index: u32,
}

fn required_dom_string_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    missing_message: &'static str,
) -> Option<Vec<u16>> {
    if args.length() <= index {
        webidl::throw_type_error(scope, missing_message);
        return None;
    }
    v8_value_to_dom_string_u16(scope, args.get(index), false).map(|value| value.into_vec())
}

pub(super) fn storage_get_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(key) = required_dom_string_arg(
        scope,
        &args,
        0,
        "Failed to execute 'getItem' on 'Storage': 1 argument required, but only 0 present.",
    ) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let value = with_storage_result(scope, args.this(), |store, origin| {
        store.try_get_item_utf16(origin, &key)
    })
    .flatten();
    match value {
        Some(v) => {
            if let Some(s) = v8_string_from_utf16_units(scope, &v) {
                rv.set(s.into());
            }
        }
        None => rv.set(v8::null(scope).into()),
    }
}

pub(super) fn storage_set_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(key) = required_dom_string_arg(
        scope,
        &args,
        0,
        "Failed to execute 'setItem' on 'Storage': 2 arguments required.",
    ) else {
        return;
    };
    let Some(value) = required_dom_string_arg(
        scope,
        &args,
        1,
        "Failed to execute 'setItem' on 'Storage': 2 arguments required.",
    ) else {
        return;
    };
    storage_put_utf16(scope, args.this(), &key, &value);
    rv.set_undefined();
}

pub(super) fn storage_remove_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(key) = required_dom_string_arg(
        scope,
        &args,
        0,
        "Failed to execute 'removeItem' on 'Storage': 1 argument required, but only 0 present.",
    ) else {
        return;
    };
    storage_remove_utf16(scope, args.this(), &key);
}

pub(super) fn storage_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    storage_clear(scope, args.this());
}

pub(super) fn storage_key_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<StorageKeyArgs>(scope, &args) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let value = with_storage_result(scope, args.this(), |store, origin| {
        store.try_key_utf16(origin, parsed.index as usize)
    })
    .flatten();
    match value {
        Some(v) => {
            if let Some(s) = v8_string_from_utf16_units(scope, &v) {
                rv.set(s.into());
            }
        }
        None => rv.set(v8::null(scope).into()),
    }
}

pub(super) fn storage_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let len =
        with_storage_result(scope, args.this(), |store, origin| store.try_len(origin)).unwrap_or(0);
    rv.set(v8::Integer::new_from_unsigned(scope, len as u32).into());
}
