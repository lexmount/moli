use crate::{
    context_bootstrap::events::{
        event_private_value, initialize_event_object_with_type, initialize_event_wrapper,
        new_event_state,
    },
    util::{
        apply_webidl_constructor_prototype_fallback, callback_data_index_value, callback_data_item,
        throw_type_error,
    },
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use webidl::WebIdlDictionary;

const ADDRESS_SLOT: &str = "__moliRtcIceErrorAddress";
const PORT_SLOT: &str = "__moliRtcIceErrorPort";
const URL_SLOT: &str = "__moliRtcIceErrorUrl";
const ERROR_CODE_SLOT: &str = "__moliRtcIceErrorCode";
const ERROR_TEXT_SLOT: &str = "__moliRtcIceErrorText";
const PAYLOAD_SLOTS: &[&str] = &[
    ADDRESS_SLOT,
    PORT_SLOT,
    URL_SLOT,
    ERROR_CODE_SLOT,
    ERROR_TEXT_SLOT,
];

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCPeerConnectionIceErrorEventInit")]
struct Init {
    // EventInit precedes derived members, which are converted lexically.
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(nullable, converter = "raw")]
    address: Option<webidl::DomString16>,
    #[webidl(required)]
    error_code: u16,
    #[webidl(default = "", converter = "usv_string")]
    error_text: String,
    #[webidl(nullable)]
    port: Option<u16>,
    #[webidl(default = "", converter = "usv_string")]
    url: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnectionIceErrorEvent")]
struct Args {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(required, with = parse_init_arg)]
    init: Init,
}

fn parse_init_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Init, webidl::WebIdlError> {
    let object = webidl::dictionary_value(
        args.get(index),
        webidl::Context::argument("RTCPeerConnectionIceErrorEvent", (index + 1) as usize),
    )?
    .ok_or_else(|| {
        webidl::WebIdlError::missing_required(webidl::Context::member(
            "RTCPeerConnectionIceErrorEventInit",
            "errorCode",
        ))
    })?;
    Init::parse_dictionary(scope, object)
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCPeerConnectionIceErrorEvent)]
struct StateDeclaration<'s> {
    #[webapi(data_property, enumerable)]
    composed: bool,
    #[webapi(slot = ADDRESS_SLOT)]
    address: v8::Local<'s, v8::Value>,
    #[webapi(slot = ERROR_CODE_SLOT)]
    error_code: u16,
    #[webapi(slot = ERROR_TEXT_SLOT)]
    error_text: v8::Local<'s, v8::String>,
    #[webapi(slot = PORT_SLOT)]
    port: v8::Local<'s, v8::Value>,
    #[webapi(slot = URL_SLOT)]
    url: v8::Local<'s, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCPeerConnectionIceErrorEvent, receiver, enumerable)]
struct PrototypeDeclaration {
    #[webapi(accessor_property, getter = payload_getter, data = callback_data_index_value(scope, 0))]
    address: (),
    #[webapi(accessor_property, getter = payload_getter, data = callback_data_index_value(scope, 1))]
    port: (),
    #[webapi(accessor_property, getter = payload_getter, data = callback_data_index_value(scope, 2))]
    url: (),
    #[webapi(accessor_property, getter = payload_getter, data = callback_data_index_value(scope, 3))]
    error_code: (),
    #[webapi(accessor_property, getter = payload_getter, data = callback_data_index_value(scope, 4))]
    error_text: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    PrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn rtc_peer_connection_ice_error_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCPeerConnectionIceErrorEvent requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<Args>(scope, &args) else {
        return;
    };
    let Some(event_type) =
        v8::String::new_from_two_byte(scope, &parsed.event_type.0, v8::NewStringType::Normal)
    else {
        return;
    };
    let address = match parsed.init.address {
        Some(address) => {
            let Some(address) =
                v8::String::new_from_two_byte(scope, &address.0, v8::NewStringType::Normal)
            else {
                return;
            };
            address.into()
        }
        None => v8::null(scope).into(),
    };
    let port = parsed
        .init
        .port
        .map(|port| v8::Number::new(scope, f64::from(port)).into())
        .unwrap_or_else(|| v8::null(scope).into());
    let Some(error_text) = v8::String::new(scope, &parsed.init.error_text) else {
        return;
    };
    let Some(url) = v8::String::new(scope, &parsed.init.url) else {
        return;
    };
    let state = new_event_state(scope);
    initialize_event_object_with_type(
        scope,
        state,
        event_type,
        parsed.init.bubbles,
        parsed.init.cancelable,
    );
    let declaration = StateDeclaration::new(
        parsed.init.composed,
        address,
        parsed.init.error_code,
        error_text,
        port,
        url,
    );
    if declaration.initialize(scope, state).is_err()
        || initialize_event_wrapper(scope, args.this(), state).is_none()
    {
        return;
    }
    apply_webidl_constructor_prototype_fallback(
        scope,
        args.this(),
        args.new_target(),
        "RTCPeerConnectionIceErrorEvent",
    );
    rv.set(args.this().into());
}

fn payload_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, PAYLOAD_SLOTS, "ICE error event attribute")
    else {
        return;
    };
    if let Some(value) = event_private_value(scope, args.this(), slot) {
        rv.set(value);
    }
}
