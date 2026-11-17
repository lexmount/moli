//! Binary member conversion after the entire WebIDL dictionary has parsed.
use super::super::{base64url, value::Text};
use moli_webidl as webidl;

pub fn decode_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    text: &Text,
    _: webidl::SnapshotPrototype,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(bytes) = base64url::decode(text) else {
        crate::embedding::throw_dom_exception_value(
            scope,
            "Invalid base64url data.",
            "EncodingError",
        );
        return None;
    };
    let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    Some(v8::ArrayBuffer::with_backing_store(scope, &store).into())
}
