use super::*;
use serde::Deserialize;

// Partial CDP schema. Serde skips omitted fields without building a Value tree.
// RemoteObject.value, deepSerializedValue and unknown extensions are opaque:
// their JSON is still forwarded, but handles inside them are not registered.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response<'a> {
    #[serde(borrow)]
    result: Option<&'a RawValue>,
    #[serde(borrow)]
    object: Option<RemoteObject<'a>>,
    #[serde(borrow)]
    objects: Option<RemoteObject<'a>>,
    #[serde(borrow)]
    exception_details: Option<ExceptionDetails<'a>>,
    #[serde(borrow)]
    internal_properties: Option<Vec<InternalProperty<'a>>>,
    #[serde(borrow)]
    private_properties: Option<Vec<PrivateProperty<'a>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteObject<'a> {
    // Borrow the string's JSON so isolated surrogates can use the existing
    // lossy UTF-16 decoder, without decoding any user value or description.
    #[serde(borrow)]
    object_id: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct ExceptionDetails<'a> {
    #[serde(borrow)]
    exception: Option<RemoteObject<'a>>,
}

#[derive(Deserialize)]
struct Property<'a> {
    #[serde(borrow)]
    value: Option<RemoteObject<'a>>,
    #[serde(borrow)]
    get: Option<RemoteObject<'a>>,
    #[serde(borrow)]
    set: Option<RemoteObject<'a>>,
    #[serde(borrow)]
    symbol: Option<RemoteObject<'a>>,
}

#[derive(Deserialize)]
struct InternalProperty<'a> {
    #[serde(borrow)]
    value: Option<RemoteObject<'a>>,
}

#[derive(Deserialize)]
struct PrivateProperty<'a> {
    #[serde(borrow)]
    value: Option<RemoteObject<'a>>,
    #[serde(borrow)]
    get: Option<RemoteObject<'a>>,
    #[serde(borrow)]
    set: Option<RemoteObject<'a>>,
}

pub(super) fn collect(message: &InspectorJson) -> serde_json::Result<Vec<String>> {
    match message {
        InspectorJson::Parsed(value) => Ok(collect_envelope(value)),
        InspectorJson::Raw(raw) => match raw.source.fields.get("result") {
            Some(result) => collect_raw_result(result),
            None => Ok(Vec::new()),
        },
    }
}

fn collect_raw_result(result: &RawValue) -> serde_json::Result<Vec<String>> {
    let response: Response<'_> = serde_json::from_str(result.get())?;
    let mut ids = Vec::new();
    if let Some(result) = response.result {
        if result.get().starts_with('[') {
            for property in serde_json::from_str::<Vec<Property<'_>>>(result.get())? {
                append_raw_objects(
                    &mut ids,
                    [property.value, property.get, property.set, property.symbol],
                )?;
            }
        } else if result.get().starts_with('{') {
            append_raw_objects(
                &mut ids,
                [Some(serde_json::from_str::<RemoteObject<'_>>(
                    result.get(),
                )?)],
            )?;
        }
    }
    append_raw_objects(&mut ids, [response.object, response.objects])?;
    if let Some(details) = response.exception_details {
        append_raw_objects(&mut ids, [details.exception])?;
    }
    for property in response.internal_properties.into_iter().flatten() {
        append_raw_objects(&mut ids, [property.value])?;
    }
    for property in response.private_properties.into_iter().flatten() {
        append_raw_objects(&mut ids, [property.value, property.get, property.set])?;
    }
    Ok(finish(ids))
}

fn append_raw_objects<'a>(
    ids: &mut Vec<String>,
    objects: impl IntoIterator<Item = Option<RemoteObject<'a>>>,
) -> serde_json::Result<()> {
    for object in objects.into_iter().flatten() {
        if let Some(id) = object.object_id
            && id.get().starts_with('"')
            && let Value::String(id) = decode_lossy_json(id.get().as_bytes())?
        {
            ids.push(id);
        }
    }
    Ok(())
}

pub(super) fn collect_envelope(value: &Value) -> Vec<String> {
    value.get("result").map(collect_result).unwrap_or_default()
}

pub(super) fn collect_result(response: &Value) -> Vec<String> {
    // Parsed/typed responses borrow the same known schema positions directly.
    // Do not clone or reserialize their potentially large Value payloads.
    let mut ids = Vec::new();
    if let Some(result) = response.get("result") {
        if let Some(properties) = result.as_array() {
            append_properties(&mut ids, properties, &["value", "get", "set", "symbol"]);
        } else {
            append_object(&mut ids, result);
        }
    }
    for key in ["object", "objects"] {
        if let Some(object) = response.get(key) {
            append_object(&mut ids, object);
        }
    }
    if let Some(exception) = response
        .get("exceptionDetails")
        .and_then(|details| details.get("exception"))
    {
        append_object(&mut ids, exception);
    }
    for (key, members) in [
        ("internalProperties", &["value"][..]),
        ("privateProperties", &["value", "get", "set"][..]),
    ] {
        if let Some(properties) = response.get(key).and_then(Value::as_array) {
            append_properties(&mut ids, properties, members);
        }
    }
    finish(ids)
}

fn append_properties(ids: &mut Vec<String>, properties: &[Value], members: &[&str]) {
    for property in properties {
        for key in members {
            if let Some(object) = property.get(key) {
                append_object(ids, object);
            }
        }
    }
}

fn append_object(ids: &mut Vec<String>, object: &Value) {
    if let Some(id) = object.get("objectId").and_then(Value::as_str) {
        ids.push(id.to_owned());
    }
}

fn finish(mut ids: Vec<String>) -> Vec<String> {
    ids.sort_unstable();
    ids.dedup();
    ids
}
