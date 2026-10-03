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
    InvalidQuery,
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
    let query = parse_key_or_range(scope, query_value)
        .map_err(|_| CollectionRequestArgsError::InvalidQuery)?;
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
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| CollectionRequestArgsError::InvalidQuery)?;
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

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::{parse_get_all_options, should_parse_get_all_options_value};

    #[test]
    fn boxed_primitives_use_dictionary_members_for_get_all_options() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        for expression in [
            "Object.assign(new Number(1), {count: 2, direction: 'prev'})",
            "Object.assign(new String('key'), {count: 2, direction: 'prev'})",
        ] {
            let source = v8::String::new(scope, expression).unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            let value = crate::script_execution::execute_compiled_script(scope, script).unwrap();
            assert!(should_parse_get_all_options_value(scope, value));
            let Ok((query, count, direction)) = parse_get_all_options(scope, value) else {
                panic!("boxed primitives must be converted as dictionaries");
            };
            assert!(query.is_undefined());
            assert_eq!(count, Some(2));
            assert_eq!(direction, moli_indexeddb::CursorDirection::Prev);
        }
        for expression in ["1", "'key'", "new Date(0)", "[1]", "new Uint8Array(1)"] {
            let source = v8::String::new(scope, expression).unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            let value = crate::script_execution::execute_compiled_script(scope, script).unwrap();
            assert!(
                !should_parse_get_all_options_value(scope, value),
                "{expression}"
            );
        }
    }
}
