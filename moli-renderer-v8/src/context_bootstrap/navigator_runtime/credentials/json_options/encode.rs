//! Binary member conversion after the entire WebIDL dictionary has parsed.
use super::super::{base64url, value::Text};
use crate::{native_bridge::throw_dom_exception, webidl};

pub(super) fn decode_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    text: &Text,
    _: webidl::SnapshotPrototype,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(bytes) = base64url::decode(text) else {
        throw_dom_exception(scope, "EncodingError", 0, "Invalid base64url data.");
        return None;
    };
    let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    Some(v8::ArrayBuffer::with_backing_store(scope, &store).into())
}
