//! Inspector payloads may contain escaped, unpaired UTF-16 code units.
//!
//! Route metadata is Unicode scalar text, but arbitrary JavaScript strings
//! are not. Keep the usual Value representation when possible, and retain
//! opaque JSON fields when decoding those strings into Rust would lose data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Number, Value, value::RawValue};

#[derive(Debug, Clone, Serialize)]
pub struct RawCdpMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(flatten)]
    fields: BTreeMap<String, Box<RawValue>>,
}

impl<'de> Deserialize<'de> for RawCdpMessage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize the root directly. serde's flattened Content buffer
        // cannot carry RawValue's borrowed JSON representation.
        let mut fields = BTreeMap::<String, Box<RawValue>>::deserialize(deserializer)?;
        fn take<T: DeserializeOwned, E: serde::de::Error>(
            fields: &mut BTreeMap<String, Box<RawValue>>,
            name: &str,
        ) -> Result<Option<T>, E> {
            fields
                .remove(name)
                .map(|value| serde_json::from_str(value.get()).map_err(E::custom))
                .transpose()
        }
        Ok(Self {
            id: take(&mut fields, "id")?,
            method: take(&mut fields, "method")?,
            session_id: take(&mut fields, "sessionId")?,
            fields,
        })
    }
}

impl PartialEq for RawCdpMessage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.method == other.method
            && self.session_id == other.session_id
            && self.fields.len() == other.fields.len()
            && self.fields.iter().all(|(key, value)| {
                other
                    .fields
                    .get(key)
                    .is_some_and(|other| value.get() == other.get())
            })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CdpMessage {
    Json(Value),
    Raw(Box<RawCdpMessage>),
}

impl From<Value> for CdpMessage {
    fn from(value: Value) -> Self {
        Self::Json(value)
    }
}

impl std::fmt::Display for CdpMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = serde_json::to_string(self).map_err(|_| std::fmt::Error)?;
        formatter.write_str(&text)
    }
}

impl PartialEq<CdpMessage> for Value {
    fn eq(&self, other: &CdpMessage) -> bool {
        other == self
    }
}

impl PartialEq<Value> for CdpMessage {
    fn eq(&self, other: &Value) -> bool {
        matches!(self, Self::Json(value) if value == other)
    }
}

impl CdpMessage {
    pub fn parse(text: &str) -> serde_json::Result<Self> {
        match serde_json::from_str(text) {
            Ok(value) => Ok(Self::Json(value)),
            Err(_) => serde_json::from_str(text).map(|value| Self::Raw(Box::new(value))),
        }
    }

    pub fn as_json(&self) -> Option<&Value> {
        match self {
            Self::Json(value) => Some(value),
            Self::Raw(_) => None,
        }
    }

    pub fn as_json_mut(&mut self) -> Option<&mut Value> {
        match self {
            Self::Json(value) => Some(value),
            Self::Raw(_) => None,
        }
    }

    pub fn into_json(self) -> Result<Value, RawCdpMessage> {
        match self {
            Self::Json(value) => Ok(value),
            Self::Raw(value) => Err(*value),
        }
    }

    pub fn id(&self) -> Option<u64> {
        match self {
            Self::Json(value) => value.get("id").and_then(Value::as_u64),
            Self::Raw(value) => value.id.as_ref().and_then(Number::as_u64),
        }
    }

    pub fn signed_id(&self) -> Option<i64> {
        match self {
            Self::Json(value) => value.get("id").and_then(Value::as_i64),
            Self::Raw(value) => value.id.as_ref().and_then(Number::as_i64),
        }
    }

    pub fn method(&self) -> Option<&str> {
        match self {
            Self::Json(value) => value.get("method").and_then(Value::as_str),
            Self::Raw(value) => value.method.as_deref(),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Json(value) => value.get("sessionId").and_then(Value::as_str),
            Self::Raw(value) => value.session_id.as_deref(),
        }
    }

    pub fn has_field(&self, field: &str) -> bool {
        match self {
            Self::Json(value) => value.get(field).is_some(),
            Self::Raw(value) => match field {
                "id" => value.id.is_some(),
                "method" => value.method.is_some(),
                "sessionId" => value.session_id.is_some(),
                _ => value.fields.contains_key(field),
            },
        }
    }

    pub fn set_id(&mut self, id: u64) {
        match self {
            Self::Json(value) => value["id"] = Value::from(id),
            Self::Raw(value) => value.id = Some(Number::from(id)),
        }
    }

    pub fn set_session_id(&mut self, session_id: Option<&str>) {
        match self {
            Self::Json(value) => {
                if let Some(session_id) = session_id {
                    value["sessionId"] = Value::from(session_id);
                } else if let Some(object) = value.as_object_mut() {
                    object.remove("sessionId");
                }
            }
            Self::Raw(value) => value.session_id = session_id.map(str::to_owned),
        }
    }

    pub fn field<T: DeserializeOwned>(&self, field: &str) -> serde_json::Result<Option<T>> {
        match self {
            Self::Json(value) => value
                .get(field)
                .cloned()
                .map(serde_json::from_value)
                .transpose(),
            Self::Raw(value) => value
                .fields
                .get(field)
                .map(|value| serde_json::from_str(value.get()))
                .transpose(),
        }
    }

    /// Inspect metadata without interpreting JavaScript strings or object keys.
    /// The caller supplies the same nesting budget used by its Value walker.
    pub fn object_ids(&self, max_depth: usize) -> Vec<String> {
        let mut ids = Vec::new();
        match self {
            Self::Json(value) => {
                let mut pending = vec![(value, max_depth)];
                while let Some((value, depth)) = pending.pop() {
                    if depth == 0 {
                        continue;
                    }
                    match value {
                        Value::Array(values) => {
                            pending.extend(values.iter().map(|value| (value, depth - 1)))
                        }
                        Value::Object(values) => {
                            for key in ["objectId", "promiseObjectId", "errorObjectId"] {
                                if let Some(id) = values.get(key).and_then(Value::as_str) {
                                    ids.push(id.to_owned());
                                }
                            }
                            pending.extend(values.values().map(|value| (value, depth - 1)));
                        }
                        _ => {}
                    }
                }
            }
            Self::Raw(value) => {
                let mut pending: Vec<_> = value
                    .fields
                    .values()
                    .map(|value| (value.as_ref(), max_depth.saturating_sub(1)))
                    .collect();
                while let Some((value, depth)) = pending.pop() {
                    if depth == 0 {
                        continue;
                    }
                    match value.get().as_bytes().first() {
                        Some(b'{') => {
                            // Unpaired keys can occur inside returnByValue data,
                            // never as Inspector metadata field names. Such data
                            // stays opaque and is forwarded without reencoding.
                            let Ok(fields) =
                                serde_json::from_str::<BTreeMap<String, &RawValue>>(value.get())
                            else {
                                continue;
                            };
                            for key in ["objectId", "promiseObjectId", "errorObjectId"] {
                                if let Some(id) = fields
                                    .get(key)
                                    .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
                                {
                                    ids.push(id);
                                }
                            }
                            pending.extend(fields.into_values().map(|value| (value, depth - 1)));
                        }
                        Some(b'[') => {
                            if let Ok(values) = serde_json::from_str::<Vec<&RawValue>>(value.get())
                            {
                                pending.extend(values.into_iter().map(|value| (value, depth - 1)));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        ids
    }

    /// Charge retained raw payload bytes and conservatively cover envelope
    /// allocations. The ordinary Value owner supplies its existing accounting.
    pub fn retained_bytes_with(&self, json_charge: impl FnOnce(&Value) -> usize) -> usize {
        match self {
            Self::Json(value) => json_charge(value),
            Self::Raw(value) => value.fields.iter().fold(
                512usize
                    .saturating_add(value.method.as_ref().map_or(0, |s| s.capacity()))
                    .saturating_add(value.session_id.as_ref().map_or(0, |s| s.capacity())),
                |total, (key, value)| {
                    total
                        .saturating_add(256)
                        .saturating_add(key.capacity())
                        .saturating_add(value.get().len())
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_utf16_payload_survives_envelope_rewrites_and_metadata_reads() {
        let mut message = CdpMessage::parse(r#"{"id":7,"result":{"result":{"type":"string","value":"\ud800"},"objectId":"native-id","nested":{"\udc00":"\ud801"}}}"#).unwrap();
        assert!(message.as_json().is_none());
        assert_eq!(message.object_ids(128), ["native-id"]);
        message.set_id(41);
        message.set_session_id(Some("session"));
        let text = serde_json::to_string(&message).unwrap();
        assert!(text.contains(r#""value":"\ud800""#));
        assert!(text.contains(r#""\udc00":"\ud801""#));
        let decoded = CdpMessage::parse(&text).unwrap();
        assert_eq!(decoded.id(), Some(41));
        assert_eq!(decoded.session_id(), Some("session"));
    }

    #[test]
    fn valid_unicode_uses_value_and_malformed_json_is_rejected() {
        let message = CdpMessage::parse(r#"{"id":7,"result":{"value":"\ud83d\ude00"}}"#).unwrap();
        assert_eq!(message.as_json().unwrap()["result"]["value"], "😀");
        for text in [
            r#"{"id":7,"result":"\ud800",}"#,
            r#"{"method":4,"params":"\ud800"}"#,
            r#"{"params":"\uG000"}"#,
        ] {
            assert!(CdpMessage::parse(text).is_err(), "{text}");
        }
    }
}
