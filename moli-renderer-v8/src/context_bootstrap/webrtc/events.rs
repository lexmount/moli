use crate::web_api_interfaces;
use crate::{
    context_bootstrap::events::{
        EventInit, event_private_value, initialize_event_object_with_type,
        initialize_event_wrapper, new_event_state,
    },
    util::{callback_data_index_value, callback_data_item, throw_type_error},
    webidl::{self, WebIdlDictionarySerialize},
};
use moli_webapi_declare::{
    WebApiFunctionTemplate, WebApiInterfaceDescriptor, initialize_web_api_constructor_receiver,
};

const ICE_EVENT_CANDIDATE_SLOT: &str = "__moliRtcIceEventCandidate";
const ICE_EVENT_URL_SLOT: &str = "__moliRtcIceEventUrl";
const DATA_CHANNEL_EVENT_CHANNEL_SLOT: &str = "__moliRtcDataChannelEventChannel";
const ERROR_EVENT_ERROR_SLOT: &str = "__moliRtcErrorEventError";
const EVENT_MEMBER_SLOTS: &[&str] = &[
    ICE_EVENT_CANDIDATE_SLOT,
    ICE_EVENT_URL_SLOT,
    DATA_CHANNEL_EVENT_CHANNEL_SLOT,
    ERROR_EVENT_ERROR_SLOT,
];

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCPeerConnectionIceEventInit", snapshot)]
struct IceEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(nullable, interface = web_api_interfaces::RTCIceCandidate)]
    #[webidl(snapshot_slot = ICE_EVENT_CANDIDATE_SLOT)]
    candidate: Option<v8::Local<'s, v8::Object>>,
    #[webidl(nullable, converter = "usv_string")]
    #[webidl(snapshot_slot = ICE_EVENT_URL_SLOT)]
    url: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnectionIceEvent")]
struct IceEventArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(dictionary)]
    init: IceEventInit<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCDataChannelEventInit", snapshot)]
struct DataChannelEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::RTCDataChannel)]
    #[webidl(snapshot_slot = DATA_CHANNEL_EVENT_CHANNEL_SLOT)]
    channel: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCDataChannelEvent")]
struct DataChannelEventArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(required, dictionary)]
    init: DataChannelEventInit<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCErrorEventInit", snapshot)]
struct ErrorEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::RTCError)]
    #[webidl(snapshot_slot = ERROR_EVENT_ERROR_SLOT)]
    error: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCErrorEvent")]
struct ErrorEventArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(required, dictionary)]
    init: ErrorEventInit<'s>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCPeerConnectionIceEvent, receiver, enumerable)]
struct IceEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = event_member_getter, data = callback_data_index_value(scope, 0))]
    candidate: (),
    #[webapi(accessor_property, getter = event_member_getter, data = callback_data_index_value(scope, 1))]
    url: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCDataChannelEvent, receiver, enumerable)]
struct DataChannelEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = event_member_getter, data = callback_data_index_value(scope, 2))]
    channel: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCErrorEvent, receiver, enumerable)]
struct ErrorEventPrototypeDeclaration {
    #[webapi(accessor_property, getter = event_member_getter, data = callback_data_index_value(scope, 3))]
    error: (),
}

pub(super) fn install_event_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "RTCPeerConnectionIceEvent" => {
            IceEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "RTCDataChannelEvent" => {
            DataChannelEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "RTCErrorEvent" => {
            ErrorEventPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => unreachable!("unsupported WebRTC event interface"),
    }
}

pub(in crate::context_bootstrap) fn rtc_peer_connection_ice_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCPeerConnectionIceEvent requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<IceEventArgs>(scope, &args) else {
        return;
    };
    if initialize_event(
        scope,
        args.this(),
        &parsed.event_type,
        &parsed.init.base,
        &parsed.init,
        web_api_interfaces::RTCPeerConnectionIceEvent::DESCRIPTOR,
    )
    .is_some()
    {
        rv.set(args.this().into());
    }
}

pub(in crate::context_bootstrap) fn rtc_data_channel_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCDataChannelEvent requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<DataChannelEventArgs>(scope, &args) else {
        return;
    };
    if initialize_event(
        scope,
        args.this(),
        &parsed.event_type,
        &parsed.init.base,
        &parsed.init,
        web_api_interfaces::RTCDataChannelEvent::DESCRIPTOR,
    )
    .is_some()
    {
        rv.set(args.this().into());
    }
}

pub(in crate::context_bootstrap) fn rtc_error_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "RTCErrorEvent requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ErrorEventArgs>(scope, &args) else {
        return;
    };
    if initialize_event(
        scope,
        args.this(),
        &parsed.event_type,
        &parsed.init.base,
        &parsed.init,
        web_api_interfaces::RTCErrorEvent::DESCRIPTOR,
    )
    .is_some()
    {
        rv.set(args.this().into());
    }
}

fn initialize_event<'s, T: WebIdlDictionarySerialize<'s>>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    event_type: &webidl::DomString16,
    init: &EventInit,
    payload: &T,
    interface: WebApiInterfaceDescriptor,
) -> Option<()> {
    if !initialize_web_api_constructor_receiver(scope, wrapper, interface.name()) {
        return None;
    }
    let event_type =
        v8::String::new_from_two_byte(scope, &event_type.0, v8::NewStringType::Normal)?;
    let state = new_event_state(scope);
    initialize_event_object_with_type(scope, state, event_type, init.bubbles, init.cancelable);
    payload.initialize_snapshot(scope, state)?;
    interface.initialize(scope, state).ok()?;
    initialize_event_wrapper(scope, wrapper, state)?;
    Some(())
}

fn event_member_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, EVENT_MEMBER_SLOTS, "WebRTC event attribute")
    else {
        return;
    };
    if let Some(value) = event_private_value(scope, args.this(), slot) {
        rv.set(value);
    } else {
        throw_type_error(scope, "Illegal invocation");
    }
}
