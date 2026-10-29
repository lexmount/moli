use super::super::events::xhr_is_aborted;
use super::super::*;
use super::{apply_xhr_failure, apply_xhr_response_pending_body};
use moli_webapi_declare::{WebApiObject, WebApiValue};

const DELIVERY_XHR: &str = "xhr";
const DELIVERY_GENERATION: &str = "generation";
const DELIVERY_VALUE: &str = "delivery";
const RESPONSE_STATUS: &str = "status";
const RESPONSE_STATUS_TEXT: &str = "statusText";
const RESPONSE_URL: &str = "url";
const RESPONSE_HEADERS: &str = "headers";
const RESPONSE_BODY: &str = "body";
const RESPONSE_BODY_TEXT: &str = "bodyText";
const RESPONSE_BODY_LENGTH: &str = "bodyLength";

/// Each queued job owns its complete outcome. A later open()/send() cannot
/// overwrite a shared pending payload or let an old job consume the new one.
enum XhrPendingDelivery<'s> {
    Response(XhrPendingResponse<'s>),
    Failure,
}

impl<'s> WebApiValue<'s> for XhrPendingDelivery<'s> {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        match self {
            Self::Response(response) => response.to_v8_value(scope),
            Self::Failure => Some(v8::null(scope).into()),
        }
    }
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct XhrPendingResponse<'s> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    #[webapi(slot = RESPONSE_STATUS)]
    status: u16,
    #[webapi(slot = RESPONSE_STATUS_TEXT)]
    status_text: String,
    #[webapi(slot = RESPONSE_URL)]
    url: String,
    #[webapi(slot = RESPONSE_HEADERS)]
    headers: String,
    #[webapi(slot = RESPONSE_BODY)]
    body: Option<v8::Local<'s, v8::Value>>,
    #[webapi(slot = RESPONSE_BODY_TEXT)]
    body_text: String,
    #[webapi(slot = RESPONSE_BODY_LENGTH)]
    body_length: usize,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct XhrDeliveryData<'s> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    #[webapi(slot = DELIVERY_XHR)]
    xhr: v8::Local<'s, v8::Object>,
    #[webapi(slot = DELIVERY_GENERATION)]
    generation: f64,
    #[webapi(slot = DELIVERY_VALUE)]
    delivery: XhrPendingDelivery<'s>,
}

pub(in crate::network_host::xhr) fn queue_xhr_response_delivery<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
    response: Response,
) {
    let (head, body) = response.into_body();
    let response_type = xhr_state_string_property(scope, xhr, XHR_RESPONSE_TYPE_SLOT)
        .as_deref()
        .and_then(XmlHttpRequestResponseType::parse)
        .unwrap_or(XmlHttpRequestResponseType::Default);
    let (body, body_text, body_length) = match response_type {
        XmlHttpRequestResponseType::ArrayBuffer | XmlHttpRequestResponseType::Blob => {
            let bytes = body
                .try_into_materialized_bytes()
                .expect("XHR response body should remain materialized at the delivery boundary");
            let length = bytes.len();
            let body =
                crate::network_host::network_body_source_object_from_bytes(scope, None, bytes);
            (Some(body.into()), String::new(), length)
        }
        XmlHttpRequestResponseType::Json
        | XmlHttpRequestResponseType::Document
        | XmlHttpRequestResponseType::Default
        | XmlHttpRequestResponseType::Text => {
            let (text, bytes) = body
                .try_into_lossy_materialized_text()
                .expect("XHR response body should remain materialized at the delivery boundary");
            (None, text, bytes.len())
        }
    };
    let response = XhrPendingResponse::new(
        head.status,
        head.status_text().to_owned(),
        head.final_url.to_string(),
        serde_json::to_string(&head.headers).unwrap_or_else(|_| "[]".to_owned()),
        body,
        body_text,
        body_length,
    );
    queue_xhr_delivery(scope, xhr, XhrPendingDelivery::Response(response));
}

pub(in crate::network_host::xhr) fn queue_xhr_failure_delivery<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    queue_xhr_delivery(scope, xhr, XhrPendingDelivery::Failure);
}

fn queue_xhr_delivery<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
    delivery: XhrPendingDelivery<'s>,
) {
    let generation = xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT).unwrap_or(0.0);
    let xhr = v8::Local::new(scope, xhr);
    let data = XhrDeliveryData::new(xhr, generation, delivery)
        .bind(scope)
        .expect("queued XHR delivery must bind");
    if let Some(callback) = v8::Function::builder(xhr_complete_callback)
        .data(data.into())
        .build(scope)
    {
        enqueue_host_microtask(scope, callback);
    }
}

fn xhr_complete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Object>::try_from(args.data()).expect("queued XHR delivery data");
    let xhr = v8::Local::<v8::Object>::try_from(
        get_private_value(scope, data, DELIVERY_XHR).expect("queued XHR receiver"),
    )
    .expect("queued XHR receiver must be an object");
    let generation = get_private_value(scope, data, DELIVERY_GENERATION)
        .and_then(|value| value.number_value(scope))
        .expect("queued XHR generation");
    if xhr_is_aborted(scope, xhr)
        || xhr_state_number_property(scope, xhr, XHR_OPEN_GENERATION_SLOT) != Some(generation)
    {
        return;
    }
    let delivery = get_private_value(scope, data, DELIVERY_VALUE).expect("queued XHR outcome");
    if delivery.is_null() {
        apply_xhr_failure(scope, xhr);
        return;
    }
    let response = v8::Local::<v8::Object>::try_from(delivery).expect("queued XHR response");
    let url = response_string(scope, response, RESPONSE_URL);
    let Some(final_url) = url::Url::parse(&url).ok().or_else(|| {
        let host_ptr = context_host_ptr_from_global_bridge(scope)?;
        Some(unsafe { &*host_ptr }.document_url().clone())
    }) else {
        apply_xhr_failure(scope, xhr);
        return;
    };
    let status = get_private_value(scope, response, RESPONSE_STATUS)
        .and_then(|value| value.number_value(scope))
        .expect("queued XHR status") as u16;
    let status_text = response_string(scope, response, RESPONSE_STATUS_TEXT);
    let headers = serde_json::from_str(&response_string(scope, response, RESPONSE_HEADERS))
        .unwrap_or_default();
    let body = get_private_value(scope, response, RESPONSE_BODY)
        .filter(|value| !value.is_null_or_undefined());
    let body_text = response_string(scope, response, RESPONSE_BODY_TEXT);
    let body_length = get_private_value(scope, response, RESPONSE_BODY_LENGTH)
        .and_then(|value| value.number_value(scope))
        .expect("queued XHR body length") as usize;
    apply_xhr_response_pending_body(
        scope,
        xhr,
        moli_fetch::ResponseHead {
            status_text: Some(status_text),
            final_url,
            status,
            headers,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        body,
        body_text,
        Some(body_length),
    );
}

fn response_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    name: &str,
) -> String {
    let value = get_private_value(scope, response, name).expect("queued XHR response field");
    v8::Local::<v8::String>::try_from(value)
        .expect("queued XHR response text field")
        .to_rust_string_lossy(scope)
}
