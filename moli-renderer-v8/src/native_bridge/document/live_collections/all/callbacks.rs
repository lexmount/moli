pub(super) use self::document_all_item_callback as document_all_call_callback;
use crate::native_bridge::collections;
use crate::util::{v8_string, v8str};
use crate::webidl;

use super::super::super::super::callback_arg_string;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLAllCollection.namedItem")]
struct HtmlAllCollectionNamedItemArgs {
    #[webidl(required)]
    name: String,
}

enum HtmlAllNameOrIndex {
    Index(u32),
    Name(String),
}

fn array_index_property_name(value: &str) -> Option<u32> {
    collections::array_index_property_name(value)
}

fn fresh_named_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    if collections::is_document_all_named_collection_value(scope, value) {
        return collections::build_fresh_document_all_named_collection_value(scope, value);
    }
    Some(value)
}

fn document_all_name_or_index(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Option<HtmlAllNameOrIndex> {
    if args.length() == 0 || args.get(0).is_undefined() {
        return None;
    }
    let name_or_index = callback_arg_string(scope, args, 0)?;
    Some(match array_index_property_name(&name_or_index) {
        Some(index) => HtmlAllNameOrIndex::Index(index),
        None => HtmlAllNameOrIndex::Name(name_or_index),
    })
}

fn resolve_document_all_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    items: v8::Local<'s, v8::Array>,
    named: v8::Local<'s, v8::Object>,
    name_or_index: HtmlAllNameOrIndex,
) -> Option<v8::Local<'s, v8::Value>> {
    match name_or_index {
        HtmlAllNameOrIndex::Index(index) => items
            .get_index(scope, index)
            .filter(|value| !value.is_null_or_undefined()),
        HtmlAllNameOrIndex::Name(key) => {
            let key = v8_string(scope, &key)?;
            let value = named
                .get(scope, key.into())
                .filter(|value| !value.is_null_or_undefined())?;
            fresh_named_value(scope, value)
        }
    }
}

pub(super) fn document_all_item_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_null();
        return;
    };
    let Some(items) = data
        .get(scope, v8str(scope, "items").into())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        rv.set_null();
        return;
    };
    let Some(named) = data
        .get(scope, v8str(scope, "named").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_null();
        return;
    };
    let Some(name_or_index) = document_all_name_or_index(scope, &args) else {
        rv.set_null();
        return;
    };
    match resolve_document_all_value(scope, items, named, name_or_index) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

pub(super) fn document_all_named_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_null();
        return;
    };
    let Some(named) = data
        .get(scope, v8str(scope, "named").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<HtmlAllCollectionNamedItemArgs>(scope, &args) else {
        return;
    };
    let Some(key) = v8_string(scope, &parsed.name) else {
        rv.set_null();
        return;
    };
    match named.get(scope, key.into()) {
        Some(value) if !value.is_null_or_undefined() => match fresh_named_value(scope, value) {
            Some(value) => rv.set(value),
            None => rv.set_null(),
        },
        _ => rv.set_null(),
    }
}
pub(super) fn document_all_named_collection_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) =
        collections::build_fresh_document_all_named_collection_value(scope, args.data())
    {
        rv.set(value);
    }
}
