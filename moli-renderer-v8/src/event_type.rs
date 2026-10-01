//! Event type identity is a DOMString, including unpaired UTF-16 surrogates.

use std::{
    fmt,
    hash::{Hash, Hasher},
};

use crate::{dom::native::DomStringValue, util, webidl};
use indexmap::Equivalent;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EventType(DomStringValue);

impl EventType {
    pub(crate) fn from_v8(scope: &v8::PinScope<'_, '_>, value: v8::Local<'_, v8::String>) -> Self {
        let mut units = vec![0; value.length()];
        value.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
        Self(DomStringValue::from_utf16(&units))
    }
}

impl Hash for EventType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Scalar strings share str's hash so native names can be queried without
        // allocating. Surrogate-bearing keys retain their original identity.
        self.0.as_str_lossy().hash(state);
        if self.0.as_str().is_none() {
            self.0.utf16_units().hash(state);
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str_lossy())
    }
}

impl From<&str> for EventType {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl<'s> webidl::WebIdlConverter<'s> for EventType {
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

/// A borrowed native name or an already-converted JS event type.
pub(crate) trait EventTypeKey: Hash + Equivalent<EventType> + fmt::Display {
    fn to_event_type(&self) -> EventType;
    fn as_str_lossy(&self) -> &str;
    fn as_str(&self) -> Option<&str>;
    fn is_type(&self, name: &str) -> bool;
    fn to_v8<'s>(&self, scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::String>>;
}

impl EventTypeKey for EventType {
    fn to_event_type(&self) -> EventType {
        self.clone()
    }
    fn as_str_lossy(&self) -> &str {
        self.0.as_str_lossy()
    }
    fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }
    fn is_type(&self, name: &str) -> bool {
        self.0.as_str() == Some(name)
    }
    fn to_v8<'s>(&self, scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::String>> {
        match self.0.as_str() {
            Some(value) => util::v8_string(scope, value),
            None => v8::String::new_from_two_byte(
                scope,
                &self.0.utf16_units(),
                v8::NewStringType::Normal,
            ),
        }
    }
}

impl Equivalent<EventType> for str {
    fn equivalent(&self, other: &EventType) -> bool {
        other.is_type(self)
    }
}

impl EventTypeKey for str {
    fn to_event_type(&self) -> EventType {
        self.into()
    }
    fn as_str_lossy(&self) -> &str {
        self
    }
    fn as_str(&self) -> Option<&str> {
        Some(self)
    }
    fn is_type(&self, name: &str) -> bool {
        self == name
    }
    fn to_v8<'s>(&self, scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::String>> {
        util::v8_string(scope, self)
    }
}

impl Equivalent<EventType> for String {
    fn equivalent(&self, other: &EventType) -> bool {
        self.as_str().equivalent(other)
    }
}

impl EventTypeKey for String {
    fn to_event_type(&self) -> EventType {
        self.as_str().into()
    }
    fn as_str_lossy(&self) -> &str {
        self
    }
    fn as_str(&self) -> Option<&str> {
        Some(self)
    }
    fn is_type(&self, name: &str) -> bool {
        self == name
    }
    fn to_v8<'s>(&self, scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::String>> {
        util::v8_string(scope, self)
    }
}
