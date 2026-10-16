mod init;

pub(in crate::network_host) use self::init::ParsedResponseInit;
use self::init::parse_response_init;
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

pub(crate) fn build_error_response_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let response = ResponseInstanceDeclaration::new(0.0, String::new(), false)
        .bind(scope)
        .ok()?;
    set_response_slot_string(scope, response, RESPONSE_TYPE_SLOT, "error");
    install_response_headers(scope, response, &[], None);
    install_response_body_stream(scope, response, None, None);
    mark_response_object(scope, response);
    mark_response_headers_immutable(scope, response);
    Some(response)
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
    let body = match response_body_init(scope, body_arg) {
        Ok(body) => body,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let has_body = body.is_some();

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

    let default_content_type = body.as_ref().and_then(|body| body.content_type.as_deref());
    install_response_headers(scope, obj, &init.headers, default_content_type);
    let body_stream = body.and_then(|body| body.source.into_body_stream(scope, obj));
    install_response_body_stream(scope, obj, None, body_stream);
    mark_response_object(scope, obj);
    rv.set(obj.into());
}

fn is_valid_response_status_text(value: &str) -> bool {
    value.chars().all(|ch| {
        let code = ch as u32;
        matches!(code, 0x09 | 0x20..=0x7e | 0x80..=0xff)
    })
}
