use serde_json::{Value, json};

use crate::{ClassicError, ClassicErrorCode};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(super) fn required_string<'a>(params: &'a Value, field: &str) -> Result<&'a str, ClassicError> {
    required_string_value(params.get(field), field)
}

pub(super) fn optional_array(
    params: &Value,
    field: &str,
) -> Result<Option<Vec<Value>>, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    value.as_array().cloned().map(Some).ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be an array"),
        )
    })
}

pub(super) fn classic_script_arguments(params: &Value) -> Result<Vec<Value>, ClassicError> {
    Ok(optional_array(params, "args")?
        .unwrap_or_default()
        .into_iter()
        .map(|value| json!({ "value": value }))
        .collect())
}

pub(crate) fn required_object_string<'a>(
    params: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, ClassicError> {
    required_string_value(params.get(field), field)
}

pub(crate) fn required_timeout_value(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<u64>, ClassicError> {
    let Some(value) = params.get(field) else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a non-negative integer or null"),
        ));
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(number) = value.as_number() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a non-negative integer or null"),
        ));
    };
    if let Some(value) = number.as_u64() {
        if value <= MAX_SAFE_INTEGER {
            return Ok(Some(value));
        }
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a safe integer"),
        ));
    }
    if let Some(value) = number.as_f64()
        && value.is_finite()
        && value >= 0.0
        && value <= MAX_SAFE_INTEGER as f64
        && value.fract() == 0.0
    {
        return Ok(Some(value as u64));
    }
    Err(ClassicError::new(
        ClassicErrorCode::InvalidArgument,
        format!("{field} must be a non-negative integer or null"),
    ))
}

pub(super) fn optional_object_string<'a>(
    params: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(None);
    };
    value.as_str().map(Some).ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a string"),
        )
    })
}

pub(super) fn optional_object_bool(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, ClassicError> {
    optional_bool_value(params.get(field), field)
}

pub(super) fn optional_object_expiry(
    params: &serde_json::Map<String, Value>,
) -> Result<Option<f64>, ClassicError> {
    let Some(value) = params.get("expiry") else {
        return Ok(None);
    };
    let Some(number) = value.as_number() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "expiry must be a non-negative integer",
        ));
    };
    if let Some(expiry) = number.as_u64() {
        if expiry <= MAX_SAFE_INTEGER {
            return Ok(Some(expiry as f64));
        }
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "expiry must be a safe integer",
        ));
    }
    if let Some(expiry) = number.as_f64()
        && expiry.is_finite()
        && expiry >= 0.0
        && expiry <= MAX_SAFE_INTEGER as f64
        && expiry.fract() == 0.0
    {
        return Ok(Some(expiry));
    }
    Err(ClassicError::new(
        ClassicErrorCode::InvalidArgument,
        "expiry must be a non-negative integer",
    ))
}

fn required_string_value<'a>(
    value: Option<&'a Value>,
    field: &str,
) -> Result<&'a str, ClassicError> {
    value.and_then(Value::as_str).ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a string"),
        )
    })
}

pub(super) fn optional_bool_value(
    value: Option<&Value>,
    field: &str,
) -> Result<Option<bool>, ClassicError> {
    let Some(value) = value else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a boolean"),
        )
    })
}
