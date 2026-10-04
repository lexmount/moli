use moli_webapi_declare::WebApiValue;

use crate::{util::new_null_prototype_object, webidl};

/// Preserve every DOMString code unit, including lone surrogates in labels and
/// PRF record keys. Base64url values must still be ASCII when they are decoded.
#[derive(Clone, PartialEq)]
pub(super) struct Text(pub(super) webidl::DomString16);

impl From<&str> for Text {
    fn from(value: &str) -> Self {
        Self(webidl::DomString16(value.encode_utf16().collect()))
    }
}

impl<'s> webidl::WebIdlConverter<'s> for Text {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::convert::<webidl::DomString16>(scope, value, context).map(Self)
    }
}

impl<'s> WebApiValue<'s> for Text {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        v8::String::new_from_two_byte(scope, &self.0.0, v8::NewStringType::Normal).map(Into::into)
    }
}

pub(super) struct Dictionary<T>(pub(super) T);

impl<'s, T: webidl::WebIdlDictionary<'s>> webidl::WebIdlConverter<'s> for Dictionary<T> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let object = match webidl::dictionary_value(value, context)? {
            Some(object) => object,
            // null/undefined mean an empty dictionary, not an Object whose
            // prototype could contribute author-installed dictionary members.
            None => new_null_prototype_object(scope),
        };
        T::parse_dictionary(scope, object).map(Self)
    }
}
