mod init;

pub(in crate::network_host) use self::init::{ParsedResponseInit, parse_response_init};
use self::init::{install_response_body_stream, install_response_headers, response_body_init};
use super::super::fetch_surface::{RESPONSE_BODY_USED_SLOT, mark_response_object};
use super::*;
use crate::util::throw_range_error;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Response)]
struct ResponseInstanceDeclaration {
    #[webapi(slot = RESPONSE_STATUS_SLOT)]
    status: f64,
    #[webapi(slot = RESPONSE_STATUS_TEXT_SLOT)]
    status_text: String,
    #[webapi(slot = RESPONSE_OK_SLOT)]
    ok: bool,
    #[webapi(slot = RESPONSE_URL_SLOT, init = "")]
    url: (),
    #[webapi(slot = RESPONSE_REDIRECTED_SLOT, init = false)]
    redirected: (),
    #[webapi(slot = RESPONSE_TYPE_SLOT, init = string("default"))]
    response_type: (),
    #[webapi(slot = RESPONSE_BODY_USED_SLOT, init = false)]
    body_used: (),
}

pub(crate) fn response_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Response': Please use the 'new' operator.",
        );
        return;
    }

    let obj = args.this();
    let body_arg = args.get(0);
    let body_stream = readable_stream_body_arg(scope, body_arg);
    if body_stream.is_some_and(|stream| readable_stream_body_locked(scope, stream)) {
        throw_type_error(
            scope,
            "Failed to construct 'Response': ReadableStream body is locked.",
        );
        return;
    }
    let body = match body_stream
        .is_none()
        .then(|| response_body_init(scope, body_arg))
        .transpose()
    {
        Ok(body) => body.flatten(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let has_body = body.is_some() || body_stream.is_some();
    let (body_buffer, default_content_type) = if let Some(body) = body {
        (
            set_network_body_owned_bytes(scope, obj, body.bytes),
            body.content_type,
        )
    } else {
        (None, None)
    };

    let init = match parse_response_init(scope, args.get(1)) {
        Ok(init) => init,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let status = init.status;
    if !(200..=599).contains(&status) {
        throw_range_error(
            scope,
            "Failed to construct 'Response': The status provided is outside the range [200, 599].",
        );
        return;
    }
    if has_body && matches!(status, 204 | 205 | 304) {
        throw_type_error(
            scope,
            "Failed to construct 'Response': Response with null body status cannot have body.",
        );
        return;
    }

    let status_text = init.status_text;
    if !is_valid_response_status_text(&status_text) {
        throw_type_error(
            scope,
            "Failed to construct 'Response': The statusText provided is not a valid reason phrase.",
        );
        return;
    }

    ResponseInstanceDeclaration::new(status as f64, status_text, (200..300).contains(&status))
        .initialize(scope, obj)
        .expect("Response instance declaration should initialize");

    install_response_headers(scope, obj, &init.headers, default_content_type.as_deref());
    install_response_body_stream(scope, obj, body_buffer, body_stream);
    mark_response_object(scope, obj);
    rv.set(obj.into());
}

fn readable_stream_body_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    if value.is_null_or_undefined() {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    web_api_interfaces::ReadableStream::is_instance(scope, object).then_some(object)
}

fn readable_stream_body_locked(
    scope: &mut v8::PinScope<'_, '_>,
    stream: v8::Local<'_, v8::Object>,
) -> bool {
    stream
        .get(scope, v8str(scope, "locked").into())
        .map(|value| value.boolean_value(scope))
        .unwrap_or(false)
}

fn is_valid_response_status_text(value: &str) -> bool {
    value.chars().all(|ch| {
        let code = ch as u32;
        matches!(code, 0x09 | 0x20..=0x7e | 0x80..=0xff)
    })
}
