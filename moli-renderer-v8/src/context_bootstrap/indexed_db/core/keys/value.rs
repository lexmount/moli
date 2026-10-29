use super::*;
use crate::webidl;

#[derive(Debug)]
pub(in crate::context_bootstrap::indexed_db) enum KeyConversionError {
    Invalid(&'static str),
    Terminated,
    Backend(IndexedDbError),
    Exception(v8::Global<v8::Value>),
}

impl From<IndexedDbError> for KeyConversionError {
    fn from(error: IndexedDbError) -> Self {
        Self::Backend(error)
    }
}

impl KeyConversionError {
    pub(in crate::context_bootstrap::indexed_db) fn into_value<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::Terminated => None,
            Self::Backend(error) => Some(request_error_object(scope, &error)),
            Self::Invalid(message) => Some(dom_exception_value(scope, message, "DataError")),
            Self::Exception(value) => Some(v8::Local::new(scope, value)),
        }
    }

    pub(in crate::context_bootstrap::indexed_db) fn throw(self, scope: &mut v8::PinScope<'_, '_>) {
        if let Some(value) = self.into_value(scope) {
            scope.throw_exception(value);
        }
    }
}

pub(in crate::context_bootstrap::indexed_db) fn parse_idb_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<Key>, KeyConversionError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let parsed = parse_idb_key_value(&mut scope, value);
    if scope.is_execution_terminating() || !scope.can_continue() {
        if scope.has_caught() {
            scope.rethrow();
        }
        return Err(KeyConversionError::Terminated);
    }
    // Retain the original exception without leaving a pending rethrow for
    // callers to accidentally replace while building a generic DataError.
    if let Some(exception) = scope.exception() {
        return Err(KeyConversionError::Exception(v8::Global::new(
            &scope, exception,
        )));
    }
    parsed
}

pub(in crate::context_bootstrap::indexed_db) fn require_idb_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Key> {
    match parse_idb_key(scope, value) {
        Ok(Some(key)) => Some(key),
        Ok(None) => {
            KeyConversionError::Invalid("The value is not a valid IndexedDB key.").throw(scope);
            None
        }
        Err(error) => {
            error.throw(scope);
            None
        }
    }
}

pub(super) fn parse_idb_key_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<Key>, KeyConversionError> {
    parse_idb_key_value_at_depth(scope, value, 0)
}

pub(super) fn parse_idb_key_value_at_depth<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    depth: usize,
) -> Result<Option<Key>, KeyConversionError> {
    if value.is_undefined() {
        return Ok(None);
    }
    struct ArrayFrame<'s> {
        array: v8::Local<'s, v8::Array>,
        length: u32,
        keys: Vec<Key>,
    }
    let mut frames: Vec<ArrayFrame<'s>> = Vec::new();
    let mut input = value;
    let key = 'walk: loop {
        let mut key = if let Ok(array) = v8::Local::<v8::Array>::try_from(input) {
            if frames.len() + depth >= moli_indexeddb::MAX_KEY_DEPTH {
                return Err(KeyConversionError::Invalid(
                    "IndexedDB array key exceeds the maximum nesting depth.",
                ));
            }
            // Frames already identify the current path. This also avoids a
            // second growing native allocation solely for cycle detection.
            if frames.iter().any(|frame| frame.array == array) {
                return Err(KeyConversionError::Invalid(
                    "IndexedDB array keys must not contain cycles.",
                ));
            }
            let length = array.length();
            if length != 0 {
                input = array_key_entry(scope, array, 0)?;
                reserve_vec(&mut frames, 1)?;
                frames.push(ArrayFrame {
                    array,
                    length,
                    keys: Vec::new(),
                });
                continue;
            }
            Key::Array(Vec::new())
        } else {
            scalar_idb_key(scope, input)?
        };
        loop {
            let Some(frame) = frames.last_mut() else {
                break 'walk key;
            };
            reserve_vec(&mut frame.keys, 1)?;
            frame.keys.push(key);
            let index = frame.keys.len() as u32;
            if index < frame.length {
                input = array_key_entry(scope, frame.array, index)?;
                break;
            }
            key = Key::Array(frames.pop().unwrap().keys);
        }
    };
    drop(frames);
    Ok(Some(key))
}

fn array_key_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    index: u32,
) -> Result<v8::Local<'s, v8::Value>, KeyConversionError> {
    let property = v8_string(scope, &index.to_string()).ok_or(KeyConversionError::Invalid(
        "IndexedDB key allocation failed.",
    ))?;
    if array.has_own_property(scope, property.into()) != Some(true) {
        return Err(KeyConversionError::Invalid(
            "IndexedDB array keys must not contain missing entries.",
        ));
    }
    array
        .get_index(scope, index)
        .ok_or(KeyConversionError::Invalid(
            "IndexedDB array key getter threw an exception.",
        ))
}

fn scalar_idb_key(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Key, KeyConversionError> {
    if let Ok(number) = v8::Local::<v8::Number>::try_from(value) {
        return Key::number(number.value())
            .ok_or(KeyConversionError::Invalid("NaN is not an IndexedDB key."));
    }
    if let Ok(date) = v8::Local::<v8::Date>::try_from(value) {
        let ms = date.value_of();
        return if ms.is_nan() {
            Err(KeyConversionError::Invalid(
                "Invalid Date is not an IndexedDB key.",
            ))
        } else {
            Ok(Key::Date(ms as i64))
        };
    }
    if let Ok(string) = v8::Local::<v8::String>::try_from(value) {
        return Ok(Key::String(
            crate::util::v8_string_to_u16_string(scope, string).into_vec(),
        ));
    }
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        if buffer.was_detached() {
            return Err(KeyConversionError::Invalid(
                "Detached buffers are not IndexedDB keys.",
            ));
        }
        let backing = buffer.get_backing_store();
        reject_unsupported_key_buffer(scope, &backing)?;

        let mut bytes = Vec::new();
        reserve_vec(&mut bytes, backing.byte_length())?;
        bytes.extend(backing.iter().map(|byte| byte.get()));
        return Ok(Key::Binary(bytes));
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let buffer = view.buffer(scope).ok_or(KeyConversionError::Invalid(
            "IndexedDB key buffer is unavailable.",
        ))?;
        if buffer.was_detached() {
            return Err(KeyConversionError::Invalid(
                "Detached buffer views are not IndexedDB keys.",
            ));
        }
        reject_unsupported_key_buffer(scope, &buffer.get_backing_store())?;

        let mut bytes = Vec::new();
        reserve_vec(&mut bytes, view.byte_length())?;
        bytes.resize(view.byte_length(), 0);
        let copied = view.copy_contents(&mut bytes);
        bytes.truncate(copied);
        return Ok(Key::Binary(bytes));
    }
    Err(KeyConversionError::Invalid(
        "The value is not a number, Date, string, binary, or array key.",
    ))
}

fn reject_unsupported_key_buffer(
    scope: &mut v8::PinScope<'_, '_>,
    backing: &v8::BackingStore,
) -> Result<(), KeyConversionError> {
    if backing.is_shared() || backing.is_resizable_by_user_javascript() {
        let message = v8str(
            scope,
            "IndexedDB binary keys require a fixed, unshared buffer.",
        );
        let exception = v8::Exception::type_error(scope, message);
        scope.throw_exception(exception);
        return Err(KeyConversionError::Invalid(
            "Unsupported IndexedDB key buffer.",
        ));
    }
    Ok(())
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
    try_key_to_js_value(scope, key).unwrap_or_else(|| v8::undefined(scope).into())
}

pub(in crate::context_bootstrap::indexed_db) fn try_key_to_js_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: &Key,
) -> Option<v8::Local<'s, v8::Value>> {
    match key {
        Key::String(value) => crate::util::v8_string_from_utf16_units(scope, value).map(Into::into),
        Key::Number(value) => Some(v8::Number::new(scope, value.value()).into()),
        Key::Date(value) => v8::Date::new(scope, *value as f64).map(Into::into),
        Key::Binary(bytes) => {
            crate::blob::array_buffer_from_bytes(scope, bytes.clone()).map(Into::into)
        }
        Key::Array(values) => {
            let array = v8::Array::new(scope, 0);
            for (index, value) in values.iter().enumerate() {
                let value = try_key_to_js_value(scope, value)?;
                moli_webapi_declare::define_array_data_property(
                    scope,
                    array,
                    u32::try_from(index).ok()?,
                    value,
                )?;
            }
            Some(array.into())
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

/// Grow native key/index buffers while preserving allocation errors.
pub(in crate::context_bootstrap::indexed_db) fn reserve_vec<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), IndexedDbError> {
    values
        .try_reserve(additional)
        .map_err(|error| IndexedDbError::Resource(error.to_string()))
}
