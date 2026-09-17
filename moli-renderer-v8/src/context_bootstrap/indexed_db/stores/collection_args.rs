use super::*;
use crate::webidl;
use moli_indexeddb::{GetAllOptionsCandidate, should_parse_get_all_options};

pub(in crate::context_bootstrap::indexed_db::stores) struct CollectionRequestArgs {
    pub(in crate::context_bootstrap::indexed_db::stores) query: Option<IdbKeyRangeQuery>,
    pub(in crate::context_bootstrap::indexed_db::stores) count: Option<usize>,
    pub(in crate::context_bootstrap::indexed_db::stores) direction: CursorDirection,
}

pub(in crate::context_bootstrap::indexed_db::stores) enum CollectionRequestArgsError {
    WebIdl(webidl::WebIdlError),
    Key(KeyConversionError),
}

pub(in crate::context_bootstrap::indexed_db::stores) fn parse_collection_request_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    operation_name: &'static str,
) -> Result<CollectionRequestArgs, CollectionRequestArgsError> {
    let positional_query = args.get(0);
    let parsed =
        if args.length() == 1 && should_parse_get_all_options_value(scope, positional_query) {
            parse_get_all_options(scope, positional_query)?
        } else {
            let count = parse_optional_count(scope, args.get(1), operation_name)
                .map_err(CollectionRequestArgsError::WebIdl)?;
            (positional_query, count, CursorDirection::default_next())
        };
    let (query_value, count, direction) = parsed;
    let query = parse_key_or_range(scope, query_value).map_err(CollectionRequestArgsError::Key)?;
    Ok(CollectionRequestArgs {
        query,
        count,
        direction,
    })
}

fn should_parse_get_all_options_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    should_parse_get_all_options(GetAllOptionsCandidate {
        is_object: value.is_object(),
        is_key_range: parse_key_range_from_value(scope, value).is_some(),
        is_date: value.is_date(),
        is_array: v8::Local::<v8::Array>::try_from(value).is_ok(),
        is_buffer_source: value_has_array_buffer_view_tag(value)
            || v8::Local::<v8::ArrayBufferView>::try_from(value).is_ok()
            || v8::Local::<v8::ArrayBuffer>::try_from(value).is_ok(),
    })
}

fn value_has_array_buffer_view_tag(value: v8::Local<'_, v8::Value>) -> bool {
    value.is_int8_array()
        || value.is_uint8_array()
        || value.is_uint8_clamped_array()
        || value.is_int16_array()
        || value.is_uint16_array()
        || value.is_int32_array()
        || value.is_uint32_array()
        || value.is_big_int64_array()
        || value.is_big_uint64_array()
        || value.is_float32_array()
        || value.is_float64_array()
        || value.is_data_view()
}

fn parse_get_all_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(v8::Local<'s, v8::Value>, Option<usize>, CursorDirection), CollectionRequestArgsError>
{
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| {
        CollectionRequestArgsError::Key(KeyConversionError::Invalid("Invalid getAll options."))
    })?;
    let query = webidl::property_result(
        scope,
        object,
        "query",
        webidl::Context::member("IDBGetAllOptions", "query"),
    )
    .map_err(CollectionRequestArgsError::WebIdl)?
    .unwrap_or_else(|| v8::undefined(scope).into());
    let count = webidl::property_result(
        scope,
        object,
        "count",
        webidl::Context::member("IDBGetAllOptions", "count"),
    )
    .map_err(CollectionRequestArgsError::WebIdl)?
    .filter(|value| !value.is_undefined())
    .map(|value| {
        webidl::convert::<webidl::EnforceRangeUnsignedLong>(
            scope,
            value,
            webidl::Context::member("IDBGetAllOptions", "count"),
        )
        .map(|count| Some(count.0 as usize))
    })
    .transpose()
    .map_err(CollectionRequestArgsError::WebIdl)?
    .flatten();
    let direction = webidl::property_result(
        scope,
        object,
        "direction",
        webidl::Context::member("IDBGetAllOptions", "direction"),
    )
    .map_err(CollectionRequestArgsError::WebIdl)?
    .map(|value| {
        parse_cursor_direction_with_context(
            scope,
            value,
            webidl::Context::member("IDBGetAllOptions", "direction"),
        )
    })
    .transpose()
    .map_err(CollectionRequestArgsError::WebIdl)?
    .unwrap_or_else(CursorDirection::default_next);
    Ok((query, count, direction))
}
