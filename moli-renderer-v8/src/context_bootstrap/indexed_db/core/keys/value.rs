use super::*;
use crate::webidl;

pub(in crate::context_bootstrap::indexed_db) fn parse_idb_key(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> std::result::Result<Option<Key>, &'static str> {
    parse_idb_key_with_depth(scope, value, 0)
}

fn parse_idb_key_with_depth(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    depth: usize,
) -> std::result::Result<Option<Key>, &'static str> {
    if value.is_undefined() {
        return Ok(None);
    }
    if depth > 64 {
        return Err("IndexedDB array keys are too deeply nested.");
    }
    if value.is_string() {
        if let Some(text) = value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
        {
            return Ok(Some(Key::String(text)));
        }
        return Err("IndexedDB string key conversion failed.");
    }
    if value.is_string_object() {
        if let Some(text) = value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
        {
            return Ok(Some(Key::String(text)));
        }
        return Err("IndexedDB string object key conversion failed.");
    }
    if value_has_array_buffer_view_tag(value)
        || v8::Local::<v8::ArrayBufferView>::try_from(value).is_ok()
        || v8::Local::<v8::ArrayBuffer>::try_from(value).is_ok()
    {
        return Err(
            "Only string, number, date, and array keys are supported in this IndexedDB MVP.",
        );
    }
    if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
        let mut keys = Vec::with_capacity(array.length() as usize);
        for index in 0..array.length() {
            let property =
                v8_string(scope, &index.to_string()).ok_or("IndexedDB key allocation failed.")?;
            if array.has_own_property(scope, property.into()) != Some(true) {
                return Err("IndexedDB array keys must not contain missing entries.");
            }
            let Some(entry) = array.get_index(scope, index) else {
                return Err("IndexedDB array keys must not contain missing entries.");
            };
            let Some(key) = parse_idb_key_with_depth(scope, entry, depth + 1)? else {
                return Err("IndexedDB array keys must not contain undefined entries.");
            };
            keys.push(key);
        }
        return Ok(Some(Key::Array(keys)));
    }
    if value.is_number() || value.is_number_object() {
        let number = value
            .number_value(scope)
            .ok_or("IndexedDB number key conversion failed.")?;
        return number_to_idb_key(number);
    }
    if let Ok(date) = v8::Local::<v8::Date>::try_from(value) {
        return number_to_idb_key(date.value_of());
    }
    Err("Only string and integer keys are supported in this IndexedDB MVP.")
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

fn number_to_idb_key(number: f64) -> std::result::Result<Option<Key>, &'static str> {
    if !number.is_finite() || number.fract() != 0.0 {
        return Err("Only string and integer keys are supported in this IndexedDB MVP.");
    }
    if number < -(MAX_SAFE_INTEGER as f64) || number > MAX_SAFE_INTEGER as f64 {
        return Err("Only string keys and safe integer keys are supported in this IndexedDB MVP.");
    }
    Ok(Some(Key::Integer(number as i64)))
}

pub(in crate::context_bootstrap::indexed_db) fn compare_idb_keys(left: &Key, right: &Key) -> i32 {
    match left.cmp(right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

pub(in crate::context_bootstrap::indexed_db) fn key_to_js_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: &Key,
) -> v8::Local<'s, v8::Value> {
    match key {
        Key::String(value) => v8_string(scope, value)
            .map(Into::into)
            .unwrap_or_else(|| v8::undefined(scope).into()),
        Key::Integer(value) => v8::Number::new(scope, *value as f64).into(),
        Key::Array(values) => {
            let values = values
                .iter()
                .map(|value| key_to_js_value(scope, value))
                .collect::<Vec<_>>();
            let array = crate::util::serialize_v8_array(scope, values.as_slice())
                .unwrap_or_else(|| v8::Array::new(scope, 0));
            array.into()
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn parse_optional_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    operation_name: &'static str,
) -> std::result::Result<Option<usize>, webidl::WebIdlError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let context = webidl::Context::argument(operation_name, 2);
    webidl::convert::<webidl::EnforceRangeUnsignedLong>(scope, value, context)
        .map(|count| Some(count.0 as usize))
}
