use crate::{util::v8str, webidl};

// Missing defaulted DOMStrings need no allocation during conversion. Supplied
// strings retain their V8 handles, including lone UTF-16 surrogates, instead of
// copying SDP and candidate text through Rust strings and back into V8.
#[derive(Default)]
pub(super) struct DictionaryDomString<'s>(Option<v8::Local<'s, v8::String>>);

impl<'s> DictionaryDomString<'s> {
    pub(super) fn from_value(value: v8::Local<'s, v8::String>) -> Self {
        Self(Some(value))
    }

    pub(super) fn into_value(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::String> {
        self.0.unwrap_or_else(|| v8str(scope, ""))
    }
}

impl<'s> webidl::WebIdlConverter<'s> for DictionaryDomString<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if value.is_symbol() {
            return Err(webidl::WebIdlError::cannot_convert(context, "DOMString"));
        }
        value
            .to_string(scope)
            .map(|value| Self(Some(value)))
            .ok_or_else(|| webidl::WebIdlError::pending_exception(context))
    }
}

impl<'s> moli_webapi_declare::WebApiValue<'s> for DictionaryDomString<'s> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        Some(self.0.unwrap_or_else(|| v8str(scope, "")).into())
    }
}

impl webidl::WebIdlSnapshotValue for DictionaryDomString<'_> {
    const SNAPSHOT_KIND: webidl::SnapshotKind = webidl::SnapshotKind::Scalar;
}
