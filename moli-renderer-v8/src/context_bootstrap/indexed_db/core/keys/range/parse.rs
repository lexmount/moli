use super::*;

pub(in crate::context_bootstrap::indexed_db) fn parse_key_range_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<IdbKeyRangeQuery> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if !object_bool_property(scope, object, INDEXED_DB_KEY_RANGE_MARKER_SLOT).unwrap_or(false) {
        return None;
    }
    Some(IdbKeyRangeQuery {
        lower: parse_idb_key(scope, object.get(scope, v8str(scope, "lower").into())?).ok()?,
        upper: parse_idb_key(scope, object.get(scope, v8str(scope, "upper").into())?).ok()?,
        lower_open: object_bool_property(scope, object, "lowerOpen").unwrap_or(false),
        upper_open: object_bool_property(scope, object, "upperOpen").unwrap_or(false),
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
