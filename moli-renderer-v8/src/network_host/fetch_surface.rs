use super::headers::{build_headers_object, headers_entries, mark_headers_immutable};
use super::response::{ParsedResponseInit, install_response_body_methods, parse_response_init};
use super::*;
use crate::context_bootstrap::readable_stream_disturbed;
pub(in crate::network_host) use crate::util::constructor_prototype;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
    throw_range_error,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

pub(in crate::network_host) const REQUEST_METHOD_SLOT: &str = "__lmRequestMethod";
pub(crate) const REQUEST_URL_SLOT: &str = "__lmRequestUrl";
pub(in crate::network_host) const REQUEST_HEADERS_SLOT: &str = "__lmRequestHeaders";
pub(in crate::network_host) const REQUEST_DESTINATION_SLOT: &str = "__lmRequestDestination";
pub(in crate::network_host) const REQUEST_REFERRER_SLOT: &str = "__lmRequestReferrer";
pub(in crate::network_host) const REQUEST_REFERRER_POLICY_SLOT: &str = "__lmRequestReferrerPolicy";
pub(in crate::network_host) const REQUEST_MODE_SLOT: &str = "__lmRequestMode";
pub(in crate::network_host) const REQUEST_CREDENTIALS_SLOT: &str = "__lmRequestCredentials";
pub(in crate::network_host) const REQUEST_CACHE_SLOT: &str = "__lmRequestCache";
pub(in crate::network_host) const REQUEST_REDIRECT_SLOT: &str = "__lmRequestRedirect";
pub(in crate::network_host) const REQUEST_INTEGRITY_SLOT: &str = "__lmRequestIntegrity";
pub(in crate::network_host) const REQUEST_KEEPALIVE_SLOT: &str = "__lmRequestKeepalive";
pub(in crate::network_host) const REQUEST_PRIORITY_SLOT: &str = "__lmRequestPriority";
pub(in crate::network_host) const REQUEST_SIGNAL_SLOT: &str = "__lmRequestSignal";
pub(in crate::network_host) const REQUEST_DUPLEX_SLOT: &str = "__lmRequestDuplex";
pub(in crate::network_host) const REQUEST_IS_HISTORY_NAVIGATION_SLOT: &str =
    "__lmRequestIsHistoryNavigation";
pub(in crate::network_host) const REQUEST_IS_RELOAD_NAVIGATION_SLOT: &str =
    "__lmRequestIsReloadNavigation";
pub(in crate::network_host) const REQUEST_BODY_SLOT: &str = "__lmRequestBody";
pub(in crate::network_host) const REQUEST_BODY_USED_SLOT: &str = "__lmRequestBodyUsed";
pub(crate) const RESPONSE_TYPE_SLOT: &str = "__lmResponseType";
pub(crate) const RESPONSE_URL_SLOT: &str = "__lmResponseUrl";
pub(crate) const RESPONSE_INTERNAL_URL_SLOT: &str = "__lmResponseInternalUrl";
pub(crate) const RESPONSE_INTERNAL_STATUS_SLOT: &str = "__lmResponseInternalStatus";
pub(crate) const RESPONSE_INTERNAL_STATUS_TEXT_SLOT: &str = "__lmResponseInternalStatusText";
pub(crate) const RESPONSE_INTERNAL_HEADERS_SLOT: &str = "__lmResponseInternalHeadersObject";
pub(crate) const RESPONSE_REDIRECTED_SLOT: &str = "__lmResponseRedirected";
pub(crate) const RESPONSE_STATUS_SLOT: &str = "__lmResponseStatus";
pub(crate) const RESPONSE_OK_SLOT: &str = "__lmResponseOk";
pub(crate) const RESPONSE_STATUS_TEXT_SLOT: &str = "__lmResponseStatusText";
pub(crate) const RESPONSE_HEADERS_SLOT: &str = "__lmResponseHeadersObject";
pub(crate) const RESPONSE_BODY_SLOT: &str = "__lmResponseBody";
pub(crate) const RESPONSE_BODY_USED_SLOT: &str = "__lmResponseBodyUsed";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Response, prototype = "Response")]
struct ResponseCloneShellDeclaration<'scope> {
    #[webapi(slot = RESPONSE_BODY_USED_SLOT, init = false)]
    body_used: (),
    #[webapi(slot = RESPONSE_BODY_SLOT)]
    body: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct ResponseInitObjectDeclaration<'scope> {
    status: Option<f64>,
    status_text: Option<String>,
    headers: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::Response)]
struct ResponseErrorStateDeclaration {
    #[webapi(slot = RESPONSE_TYPE_SLOT, init = string("error"))]
    response_type: (),
    #[webapi(slot = RESPONSE_STATUS_SLOT, init = 0)]
    status: (),
    #[webapi(slot = RESPONSE_OK_SLOT, init = false)]
    ok: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Request, enumerable)]
struct RequestTemplateMethodsDeclaration {
    #[webapi(method = "clone", length = 0, callback = request_clone_callback)]
    clone: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Response, enumerable)]
struct ResponseTemplateMethodsDeclaration {
    #[webapi(static_method = "error", length = 0, callback = response_static_error_callback)]
    error: (),

    #[webapi(
        static_method = "redirect",
        length = 1,
        callback = response_static_redirect_callback
    )]
    redirect: (),

    #[webapi(static_method = "json", length = 1, callback = response_static_json_callback)]
    json: (),

    #[webapi(method = "clone", length = 0, callback = response_clone_callback)]
    clone: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Request, enumerable)]
struct RequestSlotAccessorsDeclaration {
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 0))]
    method: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 1))]
    url: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 2))]
    headers: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 3))]
    destination: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 4))]
    referrer: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 5))]
    referrer_policy: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 6))]
    mode: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 7))]
    credentials: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 8))]
    cache: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 9))]
    redirect: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 10))]
    integrity: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 11))]
    keepalive: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 12))]
    signal: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 13))]
    duplex: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 14))]
    is_history_navigation: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 15))]
    is_reload_navigation: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 16))]
    body: (),
    #[webapi(accessor_property, getter = request_slot_attribute_getter_callback, data = callback_data_index_value(scope, 17))]
    body_used: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Response, enumerable)]
struct ResponseSlotAccessorsDeclaration {
    #[webapi(accessor_property, name = "type", getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 18))]
    response_type: (),
    #[webapi(accessor_property, getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 19))]
    url: (),
    #[webapi(accessor_property, getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 20))]
    redirected: (),
    #[webapi(accessor_property, getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 21))]
    status: (),
    #[webapi(accessor_property, getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 22))]
    ok: (),
    #[webapi(accessor_property, getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 23))]
    status_text: (),
    #[webapi(accessor_property, getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 24))]
    headers: (),
    #[webapi(accessor_property, getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 25))]
    body: (),
    #[webapi(accessor_property, getter = response_slot_attribute_getter_callback, data = callback_data_index_value(scope, 26))]
    body_used: (),
}

pub(in crate::network_host) fn mark_request_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    web_api_interfaces::Request::DESCRIPTOR
        .initialize(scope, object)
        .expect("Request brand declaration should initialize");
}

pub(crate) fn is_branded_request_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::Request::is_instance(scope, object)
}

pub(crate) fn request_headers_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Vec<(String, String)> {
    request_slot_object(scope, request, REQUEST_HEADERS_SLOT)
        .map(|headers| headers_entries(scope, headers))
        .unwrap_or_default()
}

pub(crate) fn request_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> String {
    request_slot_string(scope, request, REQUEST_METHOD_SLOT).unwrap_or_else(|| "GET".to_owned())
}

fn require_request_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if is_branded_request_object(scope, object) {
        return Some(object);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

pub(in crate::network_host) fn set_request_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    slot: &str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, request, slot, value);
}

pub(crate) fn set_request_destination_for_service_worker_fetch_event(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    destination: &str,
) {
    let Some(destination) = v8_string(scope, destination) else {
        return;
    };
    set_request_slot_value(scope, request, REQUEST_DESTINATION_SLOT, destination.into());
}

pub(crate) fn set_request_mode_for_service_worker_fetch_event(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    mode: &str,
) {
    let Some(mode) = v8_string(scope, mode) else {
        return;
    };
    set_request_slot_value(scope, request, REQUEST_MODE_SLOT, mode.into());
}

pub(crate) fn set_request_reload_navigation_for_service_worker_fetch_event(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    is_reload: bool,
) {
    set_request_slot_bool(scope, request, REQUEST_IS_RELOAD_NAVIGATION_SLOT, is_reload);
}

pub(in crate::network_host) fn set_request_slot_bool(
    scope: &mut v8::PinScope<'_, '_>,
    request: v8::Local<'_, v8::Object>,
    slot: &str,
    value: bool,
) {
    set_request_slot_value(scope, request, slot, v8::Boolean::new(scope, value).into());
}

pub(in crate::network_host) fn request_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, request, slot)
}

pub(in crate::network_host) fn request_slot_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    request_slot_value(scope, request, slot).is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn request_slot_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    request_slot_value(scope, request, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(in crate::network_host) fn request_slot_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    request_slot_value(scope, request, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::network_host) fn mark_response_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    web_api_interfaces::Response::DESCRIPTOR
        .initialize(scope, object)
        .expect("Response brand declaration should initialize");
}

pub(crate) fn is_branded_response_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::Response::is_instance(scope, object)
}

fn require_response_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if is_branded_response_object(scope, object) {
        return Some(object);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

pub(in crate::network_host) fn set_response_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    response: v8::Local<'_, v8::Object>,
    slot: &str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, response, slot, value);
}

pub(crate) fn set_response_slot_bool(
    scope: &mut v8::PinScope<'_, '_>,
    response: v8::Local<'_, v8::Object>,
    slot: &str,
    value: bool,
) {
    set_response_slot_value(scope, response, slot, v8::Boolean::new(scope, value).into());
}

pub(crate) fn set_response_slot_string(
    scope: &mut v8::PinScope<'_, '_>,
    response: v8::Local<'_, v8::Object>,
    slot: &str,
    value: &str,
) {
    let Some(value) = v8_string(scope, value) else {
        return;
    };
    set_response_slot_value(scope, response, slot, value.into());
}

pub(in crate::network_host) fn response_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, response, slot)
}

pub(crate) fn response_slot_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    response_slot_value(scope, response, slot).is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn response_slot_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<f64> {
    response_slot_value(scope, response, slot).and_then(|value| value.number_value(scope))
}

pub(crate) fn response_slot_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<String> {
    response_slot_value(scope, response, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(crate) fn response_slot_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    response_slot_value(scope, response, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(crate) fn consume_webassembly_streaming_response_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(response) = webassembly_streaming_response_object(scope, value) else {
        throw_type_error(
            scope,
            "An argument must be provided, which must be a Response or Promise<Response> object",
        );
        return None;
    };
    if !response_ok(scope, response) {
        throw_type_error(scope, "HTTP status code is not ok");
        return None;
    }
    if !response_content_type(scope, response)
        .is_some_and(|content_type| moli_web_mime::is_webassembly_mime(&content_type))
    {
        throw_type_error(
            scope,
            "Incorrect response MIME type. Expected 'application/wasm'.",
        );
        return None;
    }
    if body_already_used(scope, response, RESPONSE_BODY_USED_SLOT) {
        throw_type_error(
            scope,
            "Cannot compile WebAssembly.Module from an already read Response",
        );
        return None;
    }

    set_response_slot_bool(scope, response, RESPONSE_BODY_USED_SLOT, true);
    match consume_network_body_value_from_object(
        scope,
        response,
        NetworkBodyConsumptionKind::ArrayBuffer,
    ) {
        NetworkBodyConsumption::Ready(value) => Some(value),
        NetworkBodyConsumption::Pending(promise) => Some(promise.into()),
        NetworkBodyConsumption::Rejected(error) => {
            scope.throw_exception(error);
            None
        }
        NetworkBodyConsumption::Failed => {
            throw_type_error(scope, "Failed to materialize response body");
            None
        }
    }
}

fn webassembly_streaming_response_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    is_branded_response_object(scope, object).then_some(object)
}

fn response_ok<'s>(scope: &mut v8::PinScope<'s, '_>, response: v8::Local<'s, v8::Object>) -> bool {
    response_slot_bool(scope, response, RESPONSE_OK_SLOT)
}

fn response_content_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let headers = response_slot_object(scope, response, RESPONSE_HEADERS_SLOT)?;
    moli_web_mime::response_header_value(&headers_entries(scope, headers), "content-type")
}

fn request_slot_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_request_receiver(scope, args.this()) else {
        return;
    };
    let Some(slot) = callback_data_item(scope, &args, SLOT_ACCESSOR_SLOTS, "slot accessor slots")
    else {
        rv.set_undefined();
        return;
    };
    if slot == REQUEST_BODY_USED_SLOT {
        let body_used = request_slot_bool(scope, this, slot)
            || body_stream_is_disturbed(scope, this, REQUEST_BODY_SLOT, false);
        rv.set(v8::Boolean::new(scope, body_used).into());
        return;
    }
    let value =
        request_slot_value(scope, this, slot).unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

fn response_slot_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_response_receiver(scope, args.this()) else {
        return;
    };
    let Some(slot) = callback_data_item(scope, &args, SLOT_ACCESSOR_SLOTS, "slot accessor slots")
    else {
        rv.set_undefined();
        return;
    };
    if slot == RESPONSE_BODY_USED_SLOT {
        let body_used = response_slot_bool(scope, this, slot)
            || body_stream_is_disturbed(scope, this, RESPONSE_BODY_SLOT, true);
        rv.set(v8::Boolean::new(scope, body_used).into());
        return;
    }
    let value =
        response_slot_value(scope, this, slot).unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

const SLOT_ACCESSOR_SLOTS: &[&str] = &[
    REQUEST_METHOD_SLOT,
    REQUEST_URL_SLOT,
    REQUEST_HEADERS_SLOT,
    REQUEST_DESTINATION_SLOT,
    REQUEST_REFERRER_SLOT,
    REQUEST_REFERRER_POLICY_SLOT,
    REQUEST_MODE_SLOT,
    REQUEST_CREDENTIALS_SLOT,
    REQUEST_CACHE_SLOT,
    REQUEST_REDIRECT_SLOT,
    REQUEST_INTEGRITY_SLOT,
    REQUEST_KEEPALIVE_SLOT,
    REQUEST_SIGNAL_SLOT,
    REQUEST_DUPLEX_SLOT,
    REQUEST_IS_HISTORY_NAVIGATION_SLOT,
    REQUEST_IS_RELOAD_NAVIGATION_SLOT,
    REQUEST_BODY_SLOT,
    REQUEST_BODY_USED_SLOT,
    RESPONSE_TYPE_SLOT,
    RESPONSE_URL_SLOT,
    RESPONSE_REDIRECTED_SLOT,
    RESPONSE_STATUS_SLOT,
    RESPONSE_OK_SLOT,
    RESPONSE_STATUS_TEXT_SLOT,
    RESPONSE_HEADERS_SLOT,
    RESPONSE_BODY_SLOT,
    RESPONSE_BODY_USED_SLOT,
];

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Response.redirect")]
struct ResponseRedirectArgs {
    #[webidl(required, converter = "usv_string")]
    url: String,
    #[webidl(default = 302)]
    status: u16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Response.json")]
struct ResponseJsonArgs<'s> {
    #[webidl(required, converter = "raw")]
    data: v8::Local<'s, v8::Value>,
    #[webidl(with = response_json_init_arg)]
    init: ParsedResponseInit,
}

fn response_static_error_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let init = ResponseInitObjectDeclaration::new(None, None, None)
        .bind(scope)
        .expect("Response error init declaration should bind");
    let Some(response) = new_response_instance(scope, v8::null(scope).into(), init.into()) else {
        rv.set_undefined();
        return;
    };
    ResponseErrorStateDeclaration::default()
        .initialize(scope, response)
        .expect("Response error state declaration should initialize");
    if let Some(headers) = response_slot_object(scope, response, RESPONSE_HEADERS_SLOT) {
        mark_headers_immutable(scope, headers);
    }
    rv.set(response.into());
}

pub(crate) fn response_static_redirect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ResponseRedirectArgs>(scope, &args) else {
        return;
    };
    let data = args.data();
    let child_handle = callback_child_handle(scope, data);
    let base_url = callback_base_url(scope, data);
    let location_result = if let Some(base_url) = base_url {
        try_resolve_request_constructor_url_for_base(scope, &parsed.url, Some(base_url))
    } else {
        try_resolve_request_constructor_url_for_child(scope, &parsed.url, child_handle)
    };
    let Ok(location) = location_result else {
        throw_type_error(
            scope,
            "Failed to execute 'redirect' on 'Response': Invalid URL",
        );
        return;
    };
    if !matches!(parsed.status, 301 | 302 | 303 | 307 | 308) {
        throw_range_error(
            scope,
            "Failed to execute 'redirect' on 'Response': Invalid status code",
        );
        return;
    }
    let init = build_response_init_object(
        scope,
        ParsedResponseInit {
            status: parsed.status,
            status_text: String::new(),
            headers: Vec::new(),
        },
        &[("Location", location.as_str())],
    );
    let Some(response) = new_response_instance(scope, v8::null(scope).into(), init.into()) else {
        rv.set_undefined();
        return;
    };
    rv.set(response.into());
}

fn callback_base_url(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<url::Url> {
    if !value.is_string() {
        return None;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| url::Url::parse(&value).ok())
}

fn callback_child_handle(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<crate::document_runtime::DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless
            .then_some(index)
            .and_then(|index| usize::try_from(index).ok())
            .map(crate::document_runtime::DomHandle::new);
    }
    value
        .integer_value(scope)
        .filter(|index| *index >= 0)
        .and_then(|index| usize::try_from(index).ok())
        .map(crate::document_runtime::DomHandle::new)
}

fn response_static_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ResponseJsonArgs>(scope, &args) else {
        return;
    };
    let body = match response_json_body(scope, parsed.data) {
        Some(body) => body,
        None => return,
    };
    let init =
        build_response_init_object(scope, parsed.init, &[("Content-Type", "application/json")]);
    let Some(response) = new_response_instance(scope, body, init.into()) else {
        rv.set_undefined();
        return;
    };
    rv.set(response.into());
}

fn response_json_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result: Result<Option<v8::Global<v8::Value>>, Option<v8::Global<v8::Value>>> = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = try_catch.init();
        match v8::json::stringify(&scope, value) {
            Some(body) => {
                if body.to_rust_string_lossy(&scope) == "undefined" {
                    Ok(None)
                } else {
                    let body: v8::Local<'_, v8::Value> = body.into();
                    Ok(Some(v8::Global::new(&scope, body)))
                }
            }
            None if scope.has_caught() => Err(scope
                .exception()
                .map(|value| v8::Global::new(&scope, value))),
            None => Ok(None),
        }
    };
    match result {
        Ok(Some(body)) => {
            let body = v8::Local::new(scope, body);
            Some(body)
        }
        Ok(None) => {
            throw_type_error(
                scope,
                "Failed to execute 'json' on 'Response': Value is not JSON serializable.",
            );
            None
        }
        Err(Some(exception)) => {
            scope.throw_exception(v8::Local::new(scope, exception));
            None
        }
        Err(None) => {
            throw_type_error(
                scope,
                "Failed to execute 'json' on 'Response': Value is not JSON serializable.",
            );
            None
        }
    }
}

fn response_json_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ParsedResponseInit, webidl::WebIdlError> {
    if args.length() <= index {
        return Ok(ParsedResponseInit::default());
    }
    parse_response_init(scope, args.get(index))
}

fn build_response_init_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut parsed: ParsedResponseInit,
    extra_headers: &[(&str, &str)],
) -> v8::Local<'s, v8::Object> {
    for (name, value) in extra_headers {
        if !parsed
            .headers
            .iter()
            .any(|(entry_name, _): &(String, String)| entry_name.eq_ignore_ascii_case(name))
        {
            parsed
                .headers
                .push((name.to_ascii_lowercase(), (*value).to_owned()));
        }
    }

    ResponseInitObjectDeclaration::new(
        Some(parsed.status as f64),
        (!parsed.status_text.is_empty()).then_some(parsed.status_text),
        if parsed.headers.is_empty() {
            None
        } else {
            Some(build_headers_object(scope, &parsed.headers).into())
        },
    )
    .bind(scope)
    .expect("Response init declaration should bind")
}

fn new_response_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    body: v8::Local<'s, v8::Value>,
    init: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let ctor = global
        .get(scope, v8str(scope, "Response").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    ctor.new_instance(scope, &[body, init])
}

fn request_clone_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_request_receiver(scope, args.this()) else {
        return;
    };
    if body_already_used(scope, this, REQUEST_BODY_USED_SLOT) {
        throw_type_error(
            scope,
            "Failed to execute 'clone' on 'Request': body stream already used",
        );
        return;
    }
    let global = scope.get_current_context().global(scope);
    let Some(ctor) = global.get(scope, v8str(scope, "Request").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(ctor) = v8::Local::<v8::Function>::try_from(ctor) else {
        rv.set_undefined();
        return;
    };
    if let Some(clone) = ctor.new_instance(scope, &[this.into()]) {
        rv.set(clone.into());
    } else {
        rv.set_undefined();
    }
}

fn response_clone_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_response_receiver(scope, args.this()) else {
        return;
    };
    if body_already_used(scope, this, RESPONSE_BODY_USED_SLOT) {
        throw_type_error(
            scope,
            "Failed to execute 'clone' on 'Response': body stream already used",
        );
        return;
    }
    let global = scope.get_current_context().global(scope);
    let Some(ctor) = global.get(scope, v8str(scope, "Response").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(ctor) = v8::Local::<v8::Function>::try_from(ctor) else {
        rv.set_undefined();
        return;
    };

    let init = ResponseInitObjectDeclaration::new(
        response_slot_number(scope, this, RESPONSE_STATUS_SLOT),
        response_slot_string(scope, this, RESPONSE_STATUS_TEXT_SLOT),
        response_slot_value(scope, this, RESPONSE_HEADERS_SLOT),
    )
    .bind(scope)
    .expect("Response clone init declaration should bind");

    if let Some(source) = network_body_source_from_object(scope, this) {
        let clone = ResponseCloneShellDeclaration::new(None)
            .bind(scope)
            .expect("Response clone shell declaration should bind");
        copy_response_surface_slots(scope, this, clone);
        if let Some(stream) = clone_pending_network_body_stream(scope, source, clone) {
            set_response_slot_value(scope, clone, RESPONSE_BODY_SLOT, stream.into());
            mark_response_object(scope, clone);
            rv.set(clone.into());
            return;
        }
    }

    if response_slot_number(scope, this, RESPONSE_STATUS_SLOT) == Some(0.0) {
        let clone = ResponseCloneShellDeclaration::new(Some(v8::null(scope).into()))
            .bind(scope)
            .expect("Response error clone shell declaration should bind");
        copy_response_surface_slots(scope, this, clone);
        clone_filtered_response_internal_body_source(scope, this, clone);
        mark_response_object(scope, clone);
        rv.set(clone.into());
        return;
    }

    let body = match try_network_body_value_from_object(scope, this) {
        Ok(Some(body)) => body,
        Ok(None) => match clone_response_readable_stream_body(scope, this) {
            Ok(Some(body)) => body,
            Ok(None) => v8::undefined(scope).into(),
            Err(()) => {
                throw_type_error(
                    scope,
                    "Failed to execute 'clone' on 'Response': body stream already used",
                );
                return;
            }
        },
        Err(_) => {
            throw_type_error(scope, "Failed to materialize response body");
            return;
        }
    };
    if let Some(clone) = ctor.new_instance(scope, &[body, init.into()]) {
        rv.set(clone.into());
    } else {
        rv.set_undefined();
    }
}

fn copy_response_surface_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    from: v8::Local<'s, v8::Object>,
    to: v8::Local<'s, v8::Object>,
) {
    for slot in [
        RESPONSE_STATUS_SLOT,
        RESPONSE_STATUS_TEXT_SLOT,
        RESPONSE_OK_SLOT,
        RESPONSE_URL_SLOT,
        RESPONSE_INTERNAL_URL_SLOT,
        RESPONSE_INTERNAL_STATUS_SLOT,
        RESPONSE_INTERNAL_STATUS_TEXT_SLOT,
        RESPONSE_INTERNAL_HEADERS_SLOT,
        RESPONSE_REDIRECTED_SLOT,
        RESPONSE_TYPE_SLOT,
        RESPONSE_HEADERS_SLOT,
    ] {
        if let Some(value) = response_slot_value(scope, from, slot) {
            set_response_slot_value(scope, to, slot, value);
        }
    }
}

fn clone_response_readable_stream_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
) -> Result<Option<v8::Local<'s, v8::Value>>, ()> {
    let Some(value) = response_slot_value(scope, response, RESPONSE_BODY_SLOT) else {
        return Ok(None);
    };
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let Ok(stream) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(None);
    };
    if !web_api_interfaces::ReadableStream::is_instance(scope, stream) {
        return Ok(None);
    }
    if readable_stream_locked(scope, stream) {
        return Err(());
    }
    let global = scope.get_current_context().global(scope);
    let Some(tee) = global
        .get(scope, v8str(scope, "__lmTeeReadableStreamBody").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Err(());
    };
    let Some(branches) = tee.call(scope, v8::undefined(scope).into(), &[stream.into()]) else {
        return Err(());
    };
    let Ok(branches) = v8::Local::<v8::Object>::try_from(branches) else {
        return Err(());
    };
    let Some(original_branch) = branches.get_index(scope, 0) else {
        return Err(());
    };
    let Some(clone_branch) = branches.get_index(scope, 1) else {
        return Err(());
    };
    set_response_slot_value(scope, response, RESPONSE_BODY_SLOT, original_branch);
    Ok(Some(clone_branch))
}

fn readable_stream_locked(
    scope: &mut v8::PinScope<'_, '_>,
    stream: v8::Local<'_, v8::Object>,
) -> bool {
    stream
        .get(scope, v8str(scope, "locked").into())
        .map(|value| value.boolean_value(scope))
        .unwrap_or(false)
}

pub(in crate::network_host) fn new_abort_signal_for_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let ctor = global
        .get(scope, v8str(scope, "AbortController").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let controller = ctor.new_instance(scope, &[])?;
    controller.get(scope, v8str(scope, "signal").into())
}

pub(in crate::network_host) fn new_abort_signal_for_request_with_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(source) = source else {
        return new_abort_signal_for_request(scope);
    };
    let global = scope.get_current_context().global(scope);
    let abort_signal_ctor = global
        .get(scope, v8str(scope, "AbortSignal").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let any = abort_signal_ctor
        .get(scope, v8str(scope, "any").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let signals = v8::Array::new(scope, 1);
    let _ = signals.set_index(scope, 0, source.into());
    any.call(scope, abort_signal_ctor.into(), &[signals.into()])
}

pub(crate) fn install_request_bindings(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<'_, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    RequestTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
    RequestSlotAccessorsDeclaration::initialize_prototype_template(scope, prototype);
    install_response_body_methods(scope, prototype);
}

pub(crate) fn install_response_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    ResponseTemplateMethodsDeclaration::initialize_template(scope, template);
    let prototype = template.prototype_template(scope);
    ResponseTemplateMethodsDeclaration::initialize_prototype_template(scope, prototype);
    ResponseSlotAccessorsDeclaration::initialize_prototype_template(scope, prototype);
    install_response_body_methods(scope, prototype);
}

const BODY_STREAM_CONSUMER_RUNTIME_SOURCE: &str = r#"
(() => {
  const consumerName = "__lmConsumeReadableStreamBody";
  const teeName = "__lmTeeReadableStreamBody";
  if (
    Object.prototype.hasOwnProperty.call(globalThis, consumerName) &&
    Object.prototype.hasOwnProperty.call(globalThis, teeName)
  ) {
    return;
  }

  function uint8ArrayChunkBytes(chunk) {
    if (Object.prototype.toString.call(chunk) !== "[object Uint8Array]") {
      throw new TypeError("ReadableStream body chunks must be Uint8Array");
    }
    return chunk;
  }

  async function consumeReadableStreamBody(stream, kind, mimeType, onChunk) {
    const reader = stream.getReader();
    const chunks = [];
    let total = 0;
    try {
      for (;;) {
        const result = await reader.read();
        if (result.done) break;
        const chunk = uint8ArrayChunkBytes(result.value);
        if (typeof onChunk === "function") onChunk(chunk);
        chunks.push(chunk);
        total += chunk.byteLength;
      }
    } finally {
      try {
        reader.releaseLock();
      } catch (_) {}
    }

    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }

    if (kind === "arrayBuffer") return bytes.buffer;
    if (kind === "bytes") return bytes;
    if (kind === "text") return new TextDecoder().decode(bytes);
    if (kind === "json") return JSON.parse(new TextDecoder().decode(bytes));
    if (kind === "blob") return new Blob([bytes], { type: mimeType || "" });
    if (kind === "formData") {
      return new Response(bytes, {
        headers: [["Content-Type", mimeType || ""]]
      }).formData();
    }
    throw new TypeError("Unsupported body consumption kind");
  }

  function teeReadableStreamBody(stream) {
    if (typeof stream.tee === "function") {
      return stream.tee();
    }

    const reader = stream.getReader();
    const controllers = [null, null];
    let reading = false;
    let closed = false;
    let errored = false;
    let storedError;

    function closeAll() {
      if (closed) return;
      closed = true;
      for (const controller of controllers) {
        if (controller) controller.close();
      }
    }

    function errorAll(error) {
      if (errored) return;
      errored = true;
      storedError = error;
      for (const controller of controllers) {
        if (controller && typeof controller.error === "function") {
          controller.error(error);
        }
      }
    }

    function pump() {
      if (reading || closed || errored || !controllers[0] || !controllers[1]) return;
      reading = true;
      reader.read().then(
        result => {
          reading = false;
          if (result.done) {
            closeAll();
            return;
          }
          try {
            controllers[0].enqueue(result.value);
            controllers[1].enqueue(result.value);
          } catch (error) {
            errorAll(error);
            return;
          }
          if (controllers[0].desiredSize > 0 || controllers[1].desiredSize > 0) {
            pump();
          }
        },
        error => {
          reading = false;
          errorAll(error);
        }
      );
    }

    function branch(index) {
      return new ReadableStream({
        start(controller) {
          controllers[index] = controller;
          if (errored) {
            if (typeof controller.error === "function") controller.error(storedError);
          } else if (closed) {
            controller.close();
          } else {
            pump();
          }
        },
        pull() {
          pump();
        },
        cancel() {}
      });
    }

    return [branch(0), branch(1)];
  }

  Object.defineProperty(globalThis, consumerName, {
    value: consumeReadableStreamBody,
    configurable: true
  });
  Object.defineProperty(globalThis, teeName, {
    value: teeReadableStreamBody,
    configurable: true
  });
})();
"#;

pub(crate) fn initialize_fetch_realm_helpers(
    scope: &mut v8::PinScope<'_, '_>,
) -> anyhow::Result<()> {
    install_body_stream_consumer_runtime(scope)
}

fn install_body_stream_consumer_runtime(scope: &mut v8::PinScope<'_, '_>) -> anyhow::Result<()> {
    let Some(source) = v8_string(scope, BODY_STREAM_CONSUMER_RUNTIME_SOURCE) else {
        anyhow::bail!("failed to allocate Fetch body stream consumer runtime source");
    };
    let script = v8::Script::compile(scope, source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to compile Fetch body stream consumer runtime"))?;
    script
        .run(scope)
        .ok_or_else(|| anyhow::anyhow!("failed to run Fetch body stream consumer runtime"))?;
    Ok(())
}

fn body_already_used<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    if is_response_slot(slot) {
        return response_slot_bool(scope, object, slot)
            || body_stream_is_disturbed(scope, object, RESPONSE_BODY_SLOT, true);
    }
    request_slot_bool(scope, object, slot)
        || body_stream_is_disturbed(scope, object, REQUEST_BODY_SLOT, false)
}

fn body_stream_is_disturbed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    body_slot: &'static str,
    response: bool,
) -> bool {
    let body = if response {
        response_slot_object(scope, object, body_slot)
    } else {
        request_slot_object(scope, object, body_slot)
    };
    body.is_some_and(|stream| readable_stream_disturbed(scope, stream))
}

fn is_response_slot(slot: &str) -> bool {
    matches!(
        slot,
        RESPONSE_TYPE_SLOT
            | RESPONSE_URL_SLOT
            | RESPONSE_INTERNAL_URL_SLOT
            | RESPONSE_REDIRECTED_SLOT
            | RESPONSE_STATUS_SLOT
            | RESPONSE_OK_SLOT
            | RESPONSE_STATUS_TEXT_SLOT
            | RESPONSE_HEADERS_SLOT
            | RESPONSE_BODY_SLOT
            | RESPONSE_BODY_USED_SLOT
    )
}
