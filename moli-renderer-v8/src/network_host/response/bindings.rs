mod init;

use self::init::{install_response_body_stream, install_response_headers, response_body_init};
use super::super::fetch_surface::mark_response_object;
use super::*;
use crate::webidl;
use moli_fetch_types::{FetchResponseMetadata, FetchResponseType};
use moli_v8_util::new_intrinsic_interface_instance;
use moli_webapi::fetch_response::{initialize_response_values, parse_response_init};

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

    let Some(body) = prepare_response_body(scope, args.get(0)) else {
        return;
    };
    let init = match parse_response_init(scope, args.get(1)) {
        Ok(init) => init,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if init.validate(scope, body.is_some()) {
        initialize_response(
            scope,
            args.this(),
            init,
            body,
            &FetchResponseMetadata::default(),
        );
        rv.set(args.this().into());
    }
}

pub(in crate::network_host) fn new_response_with_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    body: v8::Local<'s, v8::Value>,
    init: ParsedResponseInit,
) -> Option<v8::Local<'s, v8::Object>> {
    let body = prepare_response_body(scope, body)?;
    if !init.validate(scope, body.is_some()) {
        return None;
    }
    let response = new_intrinsic_interface_instance(scope, "Response")?;
    initialize_response(
        scope,
        response,
        init,
        body,
        &FetchResponseMetadata::default(),
    );
    Some(response)
}

pub(crate) fn build_error_response_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let response = new_intrinsic_interface_instance(scope, "Response")?;
    let init = ParsedResponseInit {
        status: 0,
        ..ParsedResponseInit::default()
    };
    let metadata = FetchResponseMetadata {
        response_type: FetchResponseType::Error,
        timing_allow_passed: false,
        ..FetchResponseMetadata::default()
    };
    initialize_response(scope, response, init, None, &metadata);
    if let Some(headers) = response_slot_object(scope, response, RESPONSE_HEADERS_SLOT) {
        super::super::headers::mark_headers_immutable(scope, headers);
    }
    Some(response)
}

fn prepare_response_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    body: v8::Local<'s, v8::Value>,
) -> Option<Option<PreparedBodyInit>> {
    match response_body_init(scope, body) {
        Ok(body) => Some(body),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn initialize_response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    obj: v8::Local<'s, v8::Object>,
    init: ParsedResponseInit,
    body: Option<PreparedBodyInit>,
    metadata: &FetchResponseMetadata,
) {
    initialize_response_values(scope, obj, init.status, init.status_text, metadata);
    let default_content_type = body.as_ref().and_then(|body| body.content_type.as_deref());
    install_response_headers(scope, obj, &init.headers, default_content_type);
    let body_stream = body.and_then(|body| body.source.into_body_stream(scope, obj));
    install_response_body_stream(scope, obj, None, body_stream);
    mark_response_object(scope, obj);
}
