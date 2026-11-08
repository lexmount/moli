use crate::webidl;

pub(super) struct TransactionStoreNames(pub(super) Vec<moli_indexeddb::IndexedDbName>);

impl<'s> webidl::WebIdlConverter<'s> for TransactionStoreNames {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Some(names) = webidl::convert_optional_sequence::<webidl::DomString16>(
            scope,
            value,
            context,
            &webidl::StringOptions::default(),
        )? {
            return Ok(Self(
                names
                    .0
                    .into_iter()
                    .map(|name| moli_indexeddb::IndexedDbName::from_utf16(name.0))
                    .collect(),
            ));
        }
        webidl::convert::<webidl::DomString16>(scope, value, context)
            .map(|value| Self(vec![moli_indexeddb::IndexedDbName::from_utf16(value.0)]))
    }
}
