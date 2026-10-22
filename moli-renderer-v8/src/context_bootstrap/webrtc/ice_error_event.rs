use crate::{
    context_bootstrap::events::{
        EventInit, event_private_value, initialize_event_object_with_type,
        initialize_event_wrapper, new_event_state,
    },
    util::{callback_data_index_value, callback_data_item, throw_type_error},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::WebApiFunctionTemplate;
use webidl::WebIdlDictionarySerialize;

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
#[webidl(prefix = "RTCPeerConnectionIceErrorEventInit", snapshot)]
struct Init {
    // EventInit precedes derived members, which are converted lexically.
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, converter = "raw")]
    #[webidl(snapshot_slot = ADDRESS_SLOT)]
    address: Option<webidl::DomString16>,
    #[webidl(required)]
    #[webidl(snapshot_slot = ERROR_CODE_SLOT)]
    error_code: u16,
    #[webidl(default = "", converter = "usv_string")]
    #[webidl(snapshot_slot = ERROR_TEXT_SLOT)]
    error_text: String,
    #[webidl(nullable)]
    #[webidl(snapshot_slot = PORT_SLOT)]
    port: Option<u16>,
    #[webidl(default = "", converter = "usv_string")]
    #[webidl(snapshot_slot = URL_SLOT)]
    url: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnectionIceErrorEvent")]
struct Args {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(required, dictionary)]
    init: Init,
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
    let state = new_event_state(scope);
    initialize_event_object_with_type(
        scope,
        state,
        event_type,
        parsed.init.base.bubbles,
        parsed.init.base.cancelable,
    );
    if parsed.init.initialize_snapshot(scope, state).is_none()
        || web_api_interfaces::initialize(scope, state, "RTCPeerConnectionIceErrorEvent").is_err()
        || initialize_event_wrapper(scope, args.this(), state).is_none()
    {
        return;
    }
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
