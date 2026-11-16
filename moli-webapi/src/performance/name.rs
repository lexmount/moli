use super::{PERFORMANCE_ENTRY_NAME_SLOT, performance_entry_slot_value};
use moli_webapi_declare::WebApiValue;
use moli_webidl as webidl;
use std::fmt;

/// Immutable DOMString identity. Scalar names use one UTF-8 allocation; only
/// non-scalar names retain raw UTF-16, and diagnostics never participate in lookup.
#[derive(Clone, PartialEq, Eq)]
enum NameValue {
    Scalar(String),
    NonScalar(Vec<u16>),
}

#[derive(Clone, PartialEq, Eq)]
pub struct PerformanceEntryName(NameValue);

impl PerformanceEntryName {
    fn from_utf16(units: Vec<u16>) -> Self {
        match String::from_utf16(&units) {
            Ok(text) => Self(NameValue::Scalar(text)),
            Err(_) => Self(NameValue::NonScalar(units)),
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match &self.0 {
            NameValue::Scalar(text) => Some(text),
            NameValue::NonScalar(_) => None,
        }
    }
    pub fn from_entry<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        entry: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let value = v8::Local::<v8::String>::try_from(performance_entry_slot_value(
            scope,
            entry,
            PERFORMANCE_ENTRY_NAME_SLOT,
        )?)
        .ok()?;
        let mut units = vec![0; value.length()];
        value.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
        Some(Self::from_utf16(units))
    }
}
impl From<&str> for PerformanceEntryName {
    fn from(text: &str) -> Self {
        Self(NameValue::Scalar(text.to_owned()))
    }
}
impl From<String> for PerformanceEntryName {
    fn from(text: String) -> Self {
        Self(NameValue::Scalar(text))
    }
}
impl fmt::Display for PerformanceEntryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            NameValue::Scalar(text) => f.write_str(text),
            NameValue::NonScalar(units) => f.write_str(&String::from_utf16_lossy(units)),
        }
    }
}
impl<'s> webidl::WebIdlConverter<'s> for PerformanceEntryName {
    type Options = webidl::StringOptions;
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::DomString16::convert(scope, value, context, options)
            .map(|value| Self::from_utf16(value.0))
    }
}
impl<'s> WebApiValue<'s> for PerformanceEntryName {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let value = match &self.0 {
            NameValue::Scalar(text) => moli_v8_util::v8_string(scope, text),
            NameValue::NonScalar(units) => {
                v8::String::new_from_two_byte(scope, units, v8::NewStringType::Normal)
            }
        }?;
        Some(value.into())
    }
}
