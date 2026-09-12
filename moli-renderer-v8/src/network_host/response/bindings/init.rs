use super::super::*;
use crate::webidl;

pub(super) fn response_body_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    body: v8::Local<'s, v8::Value>,
) -> Result<Option<PreparedBodyInit>, webidl::WebIdlError> {
    body_init(scope, body, webidl::Context::argument("Response", 1))
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ResponseInit")]
struct ResponseInitMembers {
    #[webidl(default = 200)]
    status: u16,
    #[webidl(converter = "byte_string", default = "")]
    status_text: String,
    #[webidl(with = response_init_headers_member)]
    headers: Vec<(String, String)>,
}

pub(in crate::network_host) struct ParsedResponseInit {
    pub(in crate::network_host) status: u16,
    pub(in crate::network_host) status_text: String,
    pub(in crate::network_host) headers: Vec<(String, String)>,
}

pub(in crate::network_host) fn parse_response_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init_arg: v8::Local<'s, v8::Value>,
) -> Result<ParsedResponseInit, webidl::WebIdlError> {
    webidl::parse_dictionary::<ResponseInitMembers>(
        scope,
        init_arg,
        webidl::Context::argument("Response", 2),
    )
    .map(|init| {
        init.map(|init| ParsedResponseInit {
            status: init.status,
            status_text: init.status_text,
            headers: init.headers,
        })
        .unwrap_or_default()
    })
}

fn response_init_headers_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    _key: &str,
) -> Result<Vec<(String, String)>, webidl::WebIdlError> {
    let context = webidl::Context::member("ResponseInit", "headers");
    let Some(headers) = webidl::property_result(scope, object, "headers", context)? else {
        return Ok(Vec::new());
    };
    if headers.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    headers_entries_from_init(scope, headers)
}

impl Default for ParsedResponseInit {
    fn default() -> Self {
        Self {
            status: 200,
            status_text: String::new(),
            headers: Vec::new(),
        }
    }
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
