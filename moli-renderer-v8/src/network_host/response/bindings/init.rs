use super::super::*;
use crate::webidl;

pub(super) fn response_body_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    body: v8::Local<'s, v8::Value>,
) -> Result<Option<PreparedBodyInit>, webidl::WebIdlError> {
    body_init(scope, body, webidl::Context::argument("Response", 1))
}

pub(super) fn install_response_headers(
    scope: &mut v8::PinScope<'_, '_>,
    obj: v8::Local<'_, v8::Object>,
    headers: &[(String, String)],
    default_content_type: Option<&str>,
) {
    let mut entries = headers.to_vec();
    append_default_body_content_type(&mut entries, default_content_type);
    let entries = filter_headers_for_guard(&entries, HeadersGuard::Response);
    let headers_obj =
        build_headers_object_with_state(scope, &entries, HeadersGuard::Response, false);
    set_response_slot_value(scope, obj, RESPONSE_HEADERS_SLOT, headers_obj.into());
}

pub(super) fn install_response_body_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    body_buffer: Option<v8::Local<'s, v8::ArrayBuffer>>,
    body_stream: Option<v8::Local<'s, v8::Object>>,
) {
    let body_value = if let Some(stream) = body_stream {
        stream.into()
    } else if let Some(buffer) = body_buffer {
        new_readable_stream_from_array_buffer(scope, buffer, buffer.byte_length())
            .map(|stream| stream.into())
            .unwrap_or_else(|| v8::null(scope).into())
    } else {
        v8::null(scope).into()
    };
    set_response_slot_value(scope, obj, RESPONSE_BODY_SLOT, body_value);
}
