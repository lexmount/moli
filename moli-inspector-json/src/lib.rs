//! Inspector JSON envelopes with opaque, shared payloads.
//!
//! Routing reads only the envelope. JavaScript strings inside `result`, `error`
//! and `params` retain their JSON escapes through serialization. UTF-8 adapters
//! explicitly request a structured view; reading it never changes wire output.

use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Number, Value, value::RawValue};
use std::{
    collections::BTreeMap,
    fmt,
    ops::Deref,
    sync::{Arc, OnceLock},
};

#[derive(Clone)]
pub enum InspectorJson {
    Parsed(Value),
    Raw(Arc<RawInspectorJson>),
}

pub struct RawInspectorJson {
    source: Arc<RawInspectorSource>,
    id: Option<Number>,
    session_id: Option<String>,
    value: OnceLock<Value>,
}

struct RawInspectorSource {
    fields: BTreeMap<String, Box<RawValue>>,
    method: Option<String>,
}

/// Host metadata belongs in Parsed messages, where the CDP boundary removes it.
/// Raw Inspector messages reject these reserved parameter names at construction.
pub const PRIVATE_PROTOCOL_PARAM_FIELDS: &[&str] =
    &["__moliBlockedInterceptors", "__moliFetchRequestId"];

impl Clone for RawInspectorJson {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            id: self.id.clone(),
            session_id: self.session_id.clone(),
            value: OnceLock::new(),
        }
    }
}

impl RawInspectorJson {
    fn materialize(&self) -> Value {
        let fields = self
            .source
            .fields
            .iter()
            .map(|(key, value)| {
                decode_lossy_json(value.get().as_bytes()).map(|value| (key.clone(), value))
            })
            .collect::<serde_json::Result<serde_json::Map<_, _>>>();
        match fields {
            Ok(mut fields) => {
                if let Some(id) = self.id.as_ref() {
                    fields.insert("id".into(), Value::Number(id.clone()));
                }
                if let Some(session) = self.session_id.as_ref() {
                    fields.insert("sessionId".into(), Value::String(session.clone()));
                }
                Value::Object(fields)
            }
            Err(error) => {
                let mut reply = serde_json::json!({"error":{"code":-32603,"message":format!("Failed to decode Inspector response for UTF-8 consumer: {error}")}});
                if let Some(id) = self.id.as_ref() {
                    reply["id"] = Value::Number(id.clone());
                }
                reply
            }
        }
    }
}

impl InspectorJson {
    /// Validate an Inspector envelope and retain each field without decoding its
    /// body. Host-generated protocol messages with Moli metadata must use From<Value>.
    pub fn from_slice(bytes: &[u8]) -> serde_json::Result<Self> {
        let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(bytes)?;
        if let Some(params) = fields.get("params") {
            validate_inspector_params(params)?;
        }
        let id = fields
            .remove("id")
            .map(|value| serde_json::from_str(value.get()))
            .transpose()?;
        let session_id = fields
            .remove("sessionId")
            .map(|value| serde_json::from_str(value.get()))
            .transpose()?;
        let method = fields
            .get("method")
            .map(|value| serde_json::from_str(value.get()))
            .transpose()?;
        Ok(Self::Raw(Arc::new(RawInspectorJson {
            source: Arc::new(RawInspectorSource { fields, method }),
            id,
            session_id,
            value: OnceLock::new(),
        })))
    }

    pub fn id_i64(&self) -> Option<i64> {
        match self {
            Self::Parsed(value) => value.get("id").and_then(Value::as_i64),
            Self::Raw(raw) => raw.id.as_ref().and_then(Number::as_i64),
        }
    }

    pub fn id_u64(&self) -> Option<u64> {
        match self {
            Self::Parsed(value) => value.get("id").and_then(Value::as_u64),
            Self::Raw(raw) => raw.id.as_ref().and_then(Number::as_u64),
        }
    }

    pub fn method(&self) -> Option<&str> {
        match self {
            Self::Parsed(value) => value.get("method").and_then(Value::as_str),
            Self::Raw(raw) => raw.source.method.as_deref(),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Parsed(value) => value.get("sessionId").and_then(Value::as_str),
            Self::Raw(raw) => raw.session_id.as_deref(),
        }
    }

    pub fn has_field(&self, key: &str) -> bool {
        match self {
            Self::Parsed(value) => value.get(key).is_some(),
            Self::Raw(raw) => match key {
                "id" => raw.id.is_some(),
                "sessionId" => raw.session_id.is_some(),
                _ => raw.source.fields.contains_key(key),
            },
        }
    }

    pub fn set_id(&mut self, id: u64) {
        match self {
            Self::Parsed(value) => value["id"] = Value::from(id),
            Self::Raw(raw) => {
                let raw = Arc::make_mut(raw);
                raw.id = Some(Number::from(id));
                raw.value.take();
            }
        }
    }

    pub fn set_session_id(&mut self, session_id: Option<&str>) {
        match self {
            Self::Parsed(value) => {
                if let Some(id) = session_id {
                    value["sessionId"] = Value::from(id);
                } else if let Some(object) = value.as_object_mut() {
                    object.remove("sessionId");
                }
            }
            Self::Raw(raw) => {
                let raw = Arc::make_mut(raw);
                raw.session_id = session_id.map(str::to_owned);
                raw.value.take();
            }
        }
    }

    /// Materialize a UTF-8 business view. It is separate from the original wire
    /// fields, so loss replacement or an adapter's depth limit cannot rewrite CDP.
    pub fn value(&self) -> &Value {
        match self {
            Self::Parsed(value) => value,
            Self::Raw(raw) => raw.value.get_or_init(|| raw.materialize()),
        }
    }

    pub fn value_mut(&mut self) -> &mut Value {
        if let Self::Raw(raw) = self {
            Arc::make_mut(raw);
            let current = std::mem::replace(self, Self::Parsed(Value::Null));
            *self = Self::Parsed(current.into_value());
        }
        let Self::Parsed(value) = self else {
            unreachable!()
        };
        value
    }

    pub fn into_value(self) -> Value {
        match self {
            Self::Parsed(value) => value,
            Self::Raw(raw) => match Arc::try_unwrap(raw) {
                Ok(mut raw) => raw.value.take().unwrap_or_else(|| raw.materialize()),
                Err(raw) => Self::Raw(raw).value().clone(),
            },
        }
    }

    pub fn into_parsed(self) -> Result<Value, Self> {
        match self {
            Self::Parsed(value) => Ok(value),
            raw => Err(raw),
        }
    }

    /// Raw fields plus an already-materialized view are both retained allocations.
    pub fn retained_bytes(&self, json_charge: impl Fn(&Value) -> usize) -> usize {
        match self {
            Self::Parsed(value) => json_charge(value),
            Self::Raw(raw) => raw
                .source
                .fields
                .iter()
                .fold(
                    std::mem::size_of::<RawInspectorJson>()
                        + std::mem::size_of::<RawInspectorSource>(),
                    |size, (key, value)| {
                        size.saturating_add(key.len())
                            .saturating_add(value.get().len())
                            .saturating_add(128)
                    },
                )
                .saturating_add(raw.source.method.as_ref().map_or(0, String::len))
                .saturating_add(raw.session_id.as_ref().map_or(0, String::len))
                .saturating_add(raw.value.get().map_or(0, json_charge)),
        }
    }

    pub fn is_materialized(&self) -> bool {
        match self {
            Self::Parsed(_) => true,
            Self::Raw(raw) => raw.value.get().is_some(),
        }
    }
}

fn validate_inspector_params(params: &RawValue) -> serde_json::Result<()> {
    let text = params.get();
    // An ASCII reserved name is either literal or uses a Unicode escape. This
    // conservative prefilter keeps ordinary Inspector payloads on the raw path;
    // serde_json does the actual field-name decoding, including escaped names.
    if !text.trim_start().starts_with('{')
        || (!PRIVATE_PROTOCOL_PARAM_FIELDS
            .iter()
            .any(|key| text.contains(key))
            && !text.contains(r"\u"))
    {
        return Ok(());
    }
    let fields: BTreeMap<String, &RawValue> = serde_json::from_str(text)?;
    if PRIVATE_PROTOCOL_PARAM_FIELDS
        .iter()
        .any(|key| fields.contains_key(*key))
    {
        return Err(<serde_json::Error as serde::de::Error>::custom(
            "Moli protocol metadata requires a Parsed message",
        ));
    }
    Ok(())
}

impl From<&Value> for InspectorJson {
    fn from(value: &Value) -> Self {
        Self::Parsed(value.clone())
    }
}
impl From<&InspectorJson> for InspectorJson {
    fn from(value: &InspectorJson) -> Self {
        value.clone()
    }
}
impl From<Value> for InspectorJson {
    fn from(value: Value) -> Self {
        Self::Parsed(value)
    }
}
impl Deref for InspectorJson {
    type Target = Value;
    fn deref(&self) -> &Value {
        self.value()
    }
}
impl Serialize for InspectorJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Parsed(value) => value.serialize(serializer),
            Self::Raw(raw) => {
                let mut map = serializer.serialize_map(Some(
                    raw.source.fields.len()
                        + usize::from(raw.id.is_some())
                        + usize::from(raw.session_id.is_some()),
                ))?;
                if let Some(id) = raw.id.as_ref() {
                    map.serialize_entry("id", id)?;
                }
                for (key, value) in &raw.source.fields {
                    map.serialize_entry(key, value.as_ref())?;
                }
                if let Some(session) = raw.session_id.as_ref() {
                    map.serialize_entry("sessionId", session)?;
                }
                map.end()
            }
        }
    }
}
impl fmt::Debug for InspectorJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parsed(value) => value.fmt(f),
            Self::Raw(raw) => f
                .debug_struct("InspectorJson")
                .field("id", &raw.id)
                .field("sessionId", &raw.session_id)
                .field("fields", &raw.source.fields)
                .finish(),
        }
    }
}
impl fmt::Display for InspectorJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&serde_json::to_string(self).map_err(|_| fmt::Error)?)
    }
}
// Equality compares the encoded wire representation, never the lossy business
// view. In particular, different unpaired surrogates and depth-limit failures
// must not collapse to the same value. Raw clones share their field storage.
impl PartialEq for InspectorJson {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Raw(left), Self::Raw(right)) => {
                Arc::ptr_eq(left, right)
                    || (left.id.as_ref().map(ToString::to_string)
                        == right.id.as_ref().map(ToString::to_string)
                        && left.session_id == right.session_id
                        && (Arc::ptr_eq(&left.source, &right.source)
                            || (left.source.fields.len() == right.source.fields.len()
                                && left.source.fields.iter().zip(&right.source.fields).all(
                                    |((left_key, left), (right_key, right))| {
                                        left_key == right_key && left.get() == right.get()
                                    },
                                ))))
            }
            _ => same_json_encoding(self, other),
        }
    }
}

fn same_json_encoding(left: &impl Serialize, right: &impl Serialize) -> bool {
    serde_json::to_string(left).expect("JSON serialization into a String")
        == serde_json::to_string(right).expect("JSON serialization into a String")
}

pub fn decode_lossy_json(bytes: &[u8]) -> serde_json::Result<Value> {
    match serde_json::from_slice(bytes) {
        Ok(value) => Ok(value),
        Err(error) => {
            // Keep ordinary Inspector messages on the allocation-free byte
            // path. Only retry after repairing an unpaired surrogate escape;
            // serde_json still validates the complete JSON syntax.
            let Some(repaired) = replace_unpaired_surrogate_escapes(bytes) else {
                return Err(error);
            };
            serde_json::from_slice(&repaired)
        }
    }
}

fn replace_unpaired_surrogate_escapes(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut repaired = None;
    let mut in_string = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            b'\\' if in_string => {
                let Some(unit) = escaped_utf16_unit(&bytes[index..]) else {
                    // Skip an escaped quote or backslash as a single token.
                    // Invalid escapes remain unchanged for serde_json.
                    index += 2;
                    continue;
                };
                if (0xd800..=0xdbff).contains(&unit)
                    && escaped_utf16_unit(&bytes[index + 6..])
                        .is_some_and(|next| (0xdc00..=0xdfff).contains(&next))
                {
                    index += 12;
                    continue;
                }
                if (0xd800..=0xdfff).contains(&unit) {
                    let output = repaired.get_or_insert_with(|| bytes.to_vec());
                    output[index..index + 6].copy_from_slice(br"\ufffd");
                }
                index += 6;
            }
            _ => index += 1,
        }
    }
    repaired
}

fn escaped_utf16_unit(bytes: &[u8]) -> Option<u16> {
    bytes
        .strip_prefix(br"\u")?
        .get(..4)?
        .iter()
        .try_fold(0_u16, |unit, digit| {
            char::from(*digit)
                .to_digit(16)
                .map(|digit| (unit << 4) | digit as u16)
        })
}

#[cfg(test)]
mod tests;
