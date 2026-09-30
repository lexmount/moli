use serde_json::{Value, json};

pub(crate) fn bidi_remote_value_from_devtools(
    value: moli_protocol::automation::DevToolsRemoteValue,
) -> Value {
    let moli_protocol::automation::DevToolsRemoteValue {
        value,
        handle,
        shared_id,
        node_id: _,
        backend_node_id: _,
        window_context,
        realm: _,
        remote_type,
        remote_subtype,
        unserializable_value,
        description,
        class_name,
        deep_serialized_value,
        node_value,
    } = value;
    let mut remote = if let Some(window_context) = window_context {
        json!({
            "type": "window",
            "value": {
                "context": window_context.as_str()
            }
        })
    } else if remote_subtype.as_deref() == Some("node") {
        let mut remote = json!({
            "type": "node",
        });
        if let Some(shared_id) = shared_id
            && let Some(map) = remote.as_object_mut()
        {
            map.insert("sharedId".to_owned(), json!(shared_id.into_string()));
        }
        if let Some(node_value) = node_value
            && let Some(map) = remote.as_object_mut()
        {
            map.insert("value".to_owned(), node_value);
        }
        remote
    } else if let Some(deep_serialized_value) = deep_serialized_value {
        bidi_remote_value_from_deep_serialized(deep_serialized_value)
    } else if let Some(remote_type) = remote_type.as_deref() {
        bidi_remote_value_from_devtools_metadata(
            value,
            remote_type,
            remote_subtype.as_deref(),
            unserializable_value.as_deref(),
            description.as_deref(),
            class_name.as_deref(),
        )
    } else {
        bidi_remote_value(value, None)
    };
    if let Some(handle) = handle
        && let Some(remote) = remote.as_object_mut()
    {
        remote.insert("handle".to_owned(), json!(handle.into_string()));
    }
    remote
}

pub(crate) fn bidi_remote_value_from_devtools_metadata(
    value: Value,
    remote_type: &str,
    remote_subtype: Option<&str>,
    unserializable_value: Option<&str>,
    description: Option<&str>,
    class_name: Option<&str>,
) -> Value {
    match (remote_type, remote_subtype) {
        ("undefined", _) => json!({ "type": "undefined" }),
        ("object", Some("null")) => json!({ "type": "null" }),
        ("object", Some("promise")) => json!({ "type": "promise" }),
        ("object", Some("array")) => bidi_remote_array_value(value),
        ("object", Some("regexp")) => json!({ "type": "regexp" }),
        ("object", Some("date")) => bidi_remote_date_value(value),
        ("object", Some("map")) => json!({ "type": "map" }),
        ("object", Some("set")) => json!({ "type": "set" }),
        ("object", Some("weakmap")) => json!({ "type": "weakmap" }),
        ("object", Some("weakset")) => json!({ "type": "weakset" }),
        ("object", Some("error")) => json!({ "type": "error" }),
        ("object", Some("proxy")) => json!({ "type": "proxy" }),
        ("object", Some("generator")) => json!({ "type": "generator" }),
        ("object", Some("typedarray")) => json!({ "type": "typedarray" }),
        ("object", Some("arraybuffer")) => json!({ "type": "arraybuffer" }),
        ("function", _) => json!({ "type": "function" }),
        ("symbol", _) => json!({ "type": "symbol" }),
        ("bigint", _) => {
            let value = unserializable_value
                .map(|value| value.trim_end_matches('n').to_owned())
                .or_else(|| value.as_str().map(str::to_owned))
                .unwrap_or_default();
            json!({ "type": "bigint", "value": value })
        }
        ("number", _) => {
            if let Some(unserializable_value) = unserializable_value {
                json!({ "type": "number", "value": unserializable_value })
            } else {
                bidi_remote_value(value, None)
            }
        }
        ("boolean", _) | ("string", _) => bidi_remote_value(value, None),
        ("object", _) if is_promise_remote_metadata(description, class_name) => {
            json!({ "type": "promise" })
        }
        ("object", _) => bidi_remote_object_value(value),
        _ => bidi_remote_value(value, None),
    }
}

pub(crate) fn bidi_remote_value(value: Value, handle: Option<String>) -> Value {
    let mut remote = match value {
        Value::Null => json!({ "type": "null" }),
        Value::Bool(value) => json!({ "type": "boolean", "value": value }),
        Value::Number(value) => json!({ "type": "number", "value": value }),
        Value::String(value) => json!({ "type": "string", "value": value }),
        Value::Array(value) => json!({
            "type": "array",
            "value": value
                .into_iter()
                .map(|value| bidi_remote_value(value, None))
                .collect::<Vec<_>>(),
        }),
        Value::Object(value) => json!({
            "type": "object",
            "value": value
                .into_iter()
                .map(|(key, value)| json!([key, bidi_remote_value(value, None)]))
                .collect::<Vec<_>>(),
        }),
    };
    if let Some(handle) = handle
        && let Some(remote) = remote.as_object_mut()
    {
        remote.insert("handle".to_owned(), json!(handle));
    }
    remote
}

fn bidi_remote_value_from_deep_serialized(mut value: Value) -> Value {
    normalize_deep_serialized_value_for_bidi(&mut value);
    value
}

fn normalize_deep_serialized_value_for_bidi(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.remove("weakLocalObjectReference") {
                let internal_id = reference
                    .as_u64()
                    .map(|reference| reference.to_string())
                    .or_else(|| reference.as_i64().map(|reference| reference.to_string()))
                    .or_else(|| reference.as_str().map(str::to_owned));
                if let Some(internal_id) = internal_id {
                    map.insert("internalId".to_owned(), json!(internal_id));
                }
            }
            for child in map.values_mut() {
                normalize_deep_serialized_value_for_bidi(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                normalize_deep_serialized_value_for_bidi(child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_promise_remote_metadata(description: Option<&str>, class_name: Option<&str>) -> bool {
    matches!(class_name, Some("Promise"))
        || description.is_some_and(|description| {
            description == "Promise"
                || description == "[object Promise]"
                || description.starts_with("Promise {")
        })
}

fn bidi_remote_array_value(value: Value) -> Value {
    match value {
        Value::Array(values) => json!({
            "type": "array",
            "value": values
                .into_iter()
                .map(|value| bidi_remote_value(value, None))
                .collect::<Vec<_>>(),
        }),
        _ => json!({ "type": "array" }),
    }
}

fn bidi_remote_object_value(value: Value) -> Value {
    match value {
        Value::Object(values) => json!({
            "type": "object",
            "value": values
                .into_iter()
                .map(|(key, value)| json!([key, bidi_remote_value(value, None)]))
                .collect::<Vec<_>>(),
        }),
        _ => json!({ "type": "object" }),
    }
}

fn bidi_remote_date_value(value: Value) -> Value {
    match value {
        Value::String(value) => json!({ "type": "date", "value": value }),
        _ => json!({ "type": "date" }),
    }
}
