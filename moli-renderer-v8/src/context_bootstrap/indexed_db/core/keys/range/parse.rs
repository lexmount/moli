use super::*;
use crate::{util::get_private_value, web_api_interfaces};

pub(in crate::context_bootstrap::indexed_db) fn parse_key_range_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<IdbKeyRangeQuery> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if !web_api_interfaces::IDBKeyRange::is_instance(scope, object) {
        return None;
    }
    // get_private_value maps undefined to None; an undefined bound is valid
    // and denotes the unbounded end of lowerBound()/upperBound().
    let lower =
        get_private_value(scope, object, LOWER).unwrap_or_else(|| v8::undefined(scope).into());
    let upper =
        get_private_value(scope, object, UPPER).unwrap_or_else(|| v8::undefined(scope).into());
    Some(IdbKeyRangeQuery {
        lower: parse_idb_key(scope, lower).ok()?,
        upper: parse_idb_key(scope, upper).ok()?,
        lower_open: get_private_value(scope, object, LOWER_OPEN)?.is_true(),
        upper_open: get_private_value(scope, object, UPPER_OPEN)?.is_true(),
    })
}

pub(in crate::context_bootstrap::indexed_db) fn parse_key_or_range<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> std::result::Result<Option<IdbKeyRangeQuery>, KeyConversionError> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    if let Some(range) = parse_key_range_from_value(scope, value) {
        return Ok(Some(range));
    }
    let Some(key) = parse_idb_key(scope, value)? else {
        return Ok(None);
    };
    Ok(Some(IdbKeyRangeQuery {
        lower: Some(key.clone()),
        upper: Some(key),
        lower_open: false,
        upper_open: false,
    }))
}
