use std::collections::BTreeMap;

use serde_json::value::RawValue;

/// Read-only view of a serialized host message, including nested Target output.
/// This view cannot be serialized or promoted to an Inspector notification;
/// opaque parameter values and host-private routing fields are not interpreted.
pub struct InspectorEnvelope<'a> {
    method: Option<String>,
    session_id: Option<String>,
    params: Option<&'a RawValue>,
}

impl<'a> InspectorEnvelope<'a> {
    pub fn from_slice(bytes: &'a [u8]) -> serde_json::Result<Self> {
        let fields: BTreeMap<String, &'a RawValue> = serde_json::from_slice(bytes)?;
        Ok(Self {
            method: fields
                .get("method")
                .map(|value| serde_json::from_str(value.get()))
                .transpose()?,
            session_id: fields
                .get("sessionId")
                .map(|value| serde_json::from_str(value.get()))
                .transpose()?,
            params: fields.get("params").copied(),
        })
    }

    pub fn method(&self) -> Option<&str> {
        self.method.as_deref()
    }
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    pub fn string_param(&self, key: &str) -> serde_json::Result<Option<String>> {
        self.params
            .map(|params| super::string_param_from_raw(params, key))
            .transpose()
            .map(Option::flatten)
    }
}
