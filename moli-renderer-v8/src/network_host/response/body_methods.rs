mod binary;
mod text_json;

use super::super::fetch_surface::{
    REQUEST_BODY_USED_SLOT, REQUEST_HEADERS_SLOT, RESPONSE_BODY_USED_SLOT, RESPONSE_HEADERS_SLOT,
};
use super::*;
use moli_web_mime::{response_blob_mime_type, response_content_type};

use self::binary::{
    response_array_buffer_callback, response_blob_callback, response_bytes_callback,
};
use self::text_json::{response_json_callback, response_text_callback};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Body", enumerable)]
struct BodyTemplateMethodsDeclaration {
    #[webapi(method = "text", length = 0, callback = response_text_callback)]
    text: (),

    #[webapi(method = "json", length = 0, callback = response_json_callback)]
    json: (),

    #[webapi(method = "arrayBuffer", length = 0, callback = response_array_buffer_callback)]
    array_buffer: (),

    #[webapi(method = "blob", length = 0, callback = response_blob_callback)]
    blob: (),

    #[webapi(method = "formData", length = 0, callback = response_form_data_callback)]
    form_data: (),

    #[webapi(method = "bytes", length = 0, callback = response_bytes_callback)]
    bytes: (),
}

pub(in crate::network_host) fn install_response_body_methods<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    BodyTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
}

fn response_form_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(consumption) = begin_body_consumption_promise(scope, &args, &mut rv) else {
        return;
    };

    let content_type = body_content_type(scope, &consumption);
    finish_body_consumption(
        scope,
        &mut rv,
        consumption,
        NetworkBodyConsumptionKind::FormData { content_type },
    );
}

struct BodyConsumptionPromise<'scope> {
    object: v8::Local<'scope, v8::Object>,
    receiver: BodyReceiver,
    resolver: v8::Local<'scope, v8::PromiseResolver>,
    promise: v8::Local<'scope, v8::Promise>,
}

#[derive(Clone, Copy)]
enum BodyReceiver {
    Request,
    Response,
}

fn begin_body_consumption_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) -> Option<BodyConsumptionPromise<'s>> {
    let object = args.this();
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return None;
    };
    let promise = resolver.get_promise(scope);
    let Some(receiver) = body_receiver(scope, object) else {
        reject_body_illegal_receiver(scope, resolver);
        rv.set(promise.into());
        return None;
    };
    if !begin_body_consumption(scope, object, receiver) {
        reject_body_already_used(scope, resolver);
        rv.set(promise.into());
        return None;
    }
    Some(BodyConsumptionPromise {
        object,
        receiver,
        resolver,
        promise,
    })
}

fn finish_body_consumption<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    consumption: BodyConsumptionPromise<'s>,
    kind: NetworkBodyConsumptionKind,
) {
    match consume_network_body_value_from_object(scope, consumption.object, kind) {
        NetworkBodyConsumption::Ready(value) => {
            let _ = consumption.resolver.resolve(scope, value);
        }
        NetworkBodyConsumption::Rejected(error) => {
            let _ = consumption.resolver.reject(scope, error);
        }
        NetworkBodyConsumption::Pending(pending) => {
            rv.set(pending.into());
            return;
        }
        NetworkBodyConsumption::Failed => {
            reject_body_materialization_failure(scope, consumption.resolver);
        }
    }
    rv.set(consumption.promise.into());
}

fn reject_body_already_used<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    reject_body_type_error(
        scope,
        resolver,
        "Failed to execute body consumption: body stream already used",
    );
}

fn body_content_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    consumption: &BodyConsumptionPromise<'s>,
) -> String {
    body_headers(scope, consumption.object, consumption.receiver)
        .as_deref()
        .and_then(response_content_type)
        .unwrap_or_default()
}

fn body_headers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    receiver: BodyReceiver,
) -> Option<Vec<(String, String)>> {
    match receiver {
        BodyReceiver::Response => response_slot_object(scope, object, RESPONSE_HEADERS_SLOT)
            .map(|headers| headers_entries(scope, headers)),
        BodyReceiver::Request => request_slot_object(scope, object, REQUEST_HEADERS_SLOT)
            .map(|headers| headers_entries(scope, headers)),
    }
}

fn response_blob_mime_type_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    consumption: &BodyConsumptionPromise<'s>,
) -> String {
    body_headers(scope, consumption.object, consumption.receiver)
        .as_deref()
        .map(response_blob_mime_type)
        .unwrap_or_default()
}

fn begin_body_consumption<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    receiver: BodyReceiver,
) -> bool {
    let Some(stream) = body_stream_object(scope, object) else {
        return true;
    };
    if readable_body_stream_unusable(scope, stream) {
        return false;
    }
    match receiver {
        BodyReceiver::Response => {
            set_response_slot_bool(scope, object, RESPONSE_BODY_USED_SLOT, true);
            true
        }
        BodyReceiver::Request => {
            set_request_slot_bool(scope, object, REQUEST_BODY_USED_SLOT, true);
            true
        }
    }
}

fn body_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<BodyReceiver> {
    if is_branded_response_object(scope, object) {
        return Some(BodyReceiver::Response);
    }
    if is_branded_request_object(scope, object) {
        return Some(BodyReceiver::Request);
    }
    None
}

fn reject_body_materialization_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    reject_body_type_error(scope, resolver, "Failed to materialize response body");
}

fn reject_body_type_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
) {
    let error = v8_string(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, error);
}

fn reject_body_illegal_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) {
    reject_body_type_error(scope, resolver, "Illegal invocation");
}
