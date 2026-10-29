use super::{PERFORMANCE_ENTRY_NAME_SLOT, performance_entry_slot_value};
use crate::{dom::native::DomStringValue, util, webidl};
use moli_webapi_declare::WebApiValue;
use std::fmt;

/// Entry names are DOMStrings, not Unicode scalar strings or URLs. Keep their
/// original UTF-16 identity through argument conversion, storage and lookup.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::context_bootstrap) struct PerformanceEntryName(DomStringValue);

impl PerformanceEntryName {
    pub(super) fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }

    pub(in crate::context_bootstrap) fn from_entry<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        entry: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let value = performance_entry_slot_value(scope, entry, PERFORMANCE_ENTRY_NAME_SLOT)?;
        let value = v8::Local::<v8::String>::try_from(value).ok()?;
        Some(Self(DomStringValue::from_utf16(
            util::v8_string_to_u16_string(scope, value).as_slice(),
        )))
    }
}

impl From<&str> for PerformanceEntryName {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for PerformanceEntryName {
    fn from(value: String) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for PerformanceEntryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The scalar view is only for diagnostic messages, never identity.
        formatter.write_str(self.0.as_str_lossy())
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
            .map(|value| Self(DomStringValue::from_utf16(&value.0)))
    }
}

impl<'s> WebApiValue<'s> for PerformanceEntryName {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        let value = match self.0.as_str() {
            Some(value) => util::v8_string(scope, value),
            None => util::v8_string_from_utf16_units(scope, &self.0.utf16_units()),
        }?;
        Some(value.into())
    }
}
