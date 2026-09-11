use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod ice_candidate;
mod ice_candidate_parser;
mod session_description;
pub(in crate::context_bootstrap) use ice_candidate::rtc_ice_candidate_constructor_callback;
pub(in crate::context_bootstrap) use session_description::rtc_session_description_constructor_callback;

const RTC_PEER_CONNECTION_CONFIGURATION_SLOT: &str = "__moliRtcPeerConnectionConfiguration";
const RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT: &str = "__moliRtcPeerConnectionSignalingState";
const RTC_PEER_CONNECTION_ICE_GATHERING_STATE_SLOT: &str =
    "__moliRtcPeerConnectionIceGatheringState";
const RTC_PEER_CONNECTION_ICE_CONNECTION_STATE_SLOT: &str =
    "__moliRtcPeerConnectionIceConnectionState";
const RTC_PEER_CONNECTION_CONNECTION_STATE_SLOT: &str = "__moliRtcPeerConnectionConnectionState";
const RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT: &str = "__moliRtcPeerConnectionLocalDescription";
const RTC_PEER_CONNECTION_CURRENT_LOCAL_DESCRIPTION_SLOT: &str =
    "__moliRtcPeerConnectionCurrentLocalDescription";
const RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT: &str =
    "__moliRtcPeerConnectionPendingLocalDescription";
const RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT: &str = "__moliRtcPeerConnectionHasDataChannel";
const RTC_PEER_CONNECTION_LISTENERS_SLOT: &str = "__moliRtcPeerConnectionListeners";

const RTC_DATA_CHANNEL_LABEL_SLOT: &str = "__moliRtcDataChannelLabel";
const RTC_DATA_CHANNEL_ORDERED_SLOT: &str = "__moliRtcDataChannelOrdered";
const RTC_DATA_CHANNEL_MAX_PACKET_LIFETIME_SLOT: &str = "__moliRtcDataChannelMaxPacketLifetime";
const RTC_DATA_CHANNEL_MAX_RETRANSMITS_SLOT: &str = "__moliRtcDataChannelMaxRetransmits";
const RTC_DATA_CHANNEL_PROTOCOL_SLOT: &str = "__moliRtcDataChannelProtocol";
const RTC_DATA_CHANNEL_NEGOTIATED_SLOT: &str = "__moliRtcDataChannelNegotiated";
const RTC_DATA_CHANNEL_ID_SLOT: &str = "__moliRtcDataChannelId";
const RTC_DATA_CHANNEL_READY_STATE_SLOT: &str = "__moliRtcDataChannelReadyState";
const RTC_DATA_CHANNEL_BUFFERED_AMOUNT_SLOT: &str = "__moliRtcDataChannelBufferedAmount";
const RTC_DATA_CHANNEL_BINARY_TYPE_SLOT: &str = "__moliRtcDataChannelBinaryType";
const RTC_DATA_CHANNEL_LISTENERS_SLOT: &str = "__moliRtcDataChannelListeners";

const RTC_PEER_CONNECTION_STATE_SLOTS: &[&str] = &[
    RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT,
    RTC_PEER_CONNECTION_ICE_GATHERING_STATE_SLOT,
    RTC_PEER_CONNECTION_ICE_CONNECTION_STATE_SLOT,
    RTC_PEER_CONNECTION_CONNECTION_STATE_SLOT,
];

const RTC_PEER_CONNECTION_DESCRIPTION_SLOTS: &[&str] = &[
    RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT,
    RTC_PEER_CONNECTION_CURRENT_LOCAL_DESCRIPTION_SLOT,
    RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT,
];

const RTC_DATA_CHANNEL_VALUE_SLOTS: &[&str] = &[
    RTC_DATA_CHANNEL_LABEL_SLOT,
    RTC_DATA_CHANNEL_ORDERED_SLOT,
    RTC_DATA_CHANNEL_MAX_PACKET_LIFETIME_SLOT,
    RTC_DATA_CHANNEL_MAX_RETRANSMITS_SLOT,
    RTC_DATA_CHANNEL_PROTOCOL_SLOT,
    RTC_DATA_CHANNEL_NEGOTIATED_SLOT,
    RTC_DATA_CHANNEL_ID_SLOT,
    RTC_DATA_CHANNEL_READY_STATE_SLOT,
    RTC_DATA_CHANNEL_BUFFERED_AMOUNT_SLOT,
    RTC_DATA_CHANNEL_BINARY_TYPE_SLOT,
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCPeerConnection)]
struct RtcPeerConnectionObjectDeclaration<'scope> {
    #[webapi(slot = RTC_PEER_CONNECTION_CONFIGURATION_SLOT)]
    configuration: v8::Local<'scope, v8::Object>,
    #[webapi(slot = RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT)]
    signaling_state: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_PEER_CONNECTION_ICE_GATHERING_STATE_SLOT)]
    ice_gathering_state: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_PEER_CONNECTION_ICE_CONNECTION_STATE_SLOT)]
    ice_connection_state: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_PEER_CONNECTION_CONNECTION_STATE_SLOT)]
    connection_state: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT, init = "null")]
    local_description: (),
    #[webapi(slot = RTC_PEER_CONNECTION_CURRENT_LOCAL_DESCRIPTION_SLOT, init = "null")]
    current_local_description: (),
    #[webapi(slot = RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT, init = "null")]
    pending_local_description: (),
    #[webapi(slot = RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT, init = false)]
    has_data_channel: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = RTC_PEER_CONNECTION_LISTENERS_SLOT)]
    event_target_slot: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCDataChannel)]
struct RtcDataChannelObjectDeclaration<'scope> {
    #[webapi(slot = RTC_DATA_CHANNEL_LABEL_SLOT)]
    label: v8::Local<'scope, v8::String>,
    #[webapi(slot = RTC_DATA_CHANNEL_ORDERED_SLOT, init = true)]
    ordered: (),
    #[webapi(slot = RTC_DATA_CHANNEL_MAX_PACKET_LIFETIME_SLOT, init = "null")]
    max_packet_lifetime: (),
    #[webapi(slot = RTC_DATA_CHANNEL_MAX_RETRANSMITS_SLOT, init = "null")]
    max_retransmits: (),
    #[webapi(slot = RTC_DATA_CHANNEL_PROTOCOL_SLOT, init = "")]
    protocol: (),
    #[webapi(slot = RTC_DATA_CHANNEL_NEGOTIATED_SLOT, init = false)]
    negotiated: (),
    #[webapi(slot = RTC_DATA_CHANNEL_ID_SLOT, init = "null")]
    id: (),
    #[webapi(slot = RTC_DATA_CHANNEL_READY_STATE_SLOT, init = string("connecting"))]
    ready_state: (),
    #[webapi(slot = RTC_DATA_CHANNEL_BUFFERED_AMOUNT_SLOT, init = 0)]
    buffered_amount: (),
    #[webapi(slot = RTC_DATA_CHANNEL_BINARY_TYPE_SLOT, init = string("arraybuffer"))]
    binary_type: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = RTC_DATA_CHANNEL_LISTENERS_SLOT)]
    event_target_slot: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiObject)]
#[webapi(record)]
struct RtcSessionDescriptionInitDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'scope, v8::String>,
    #[webapi(data_property, enumerable)]
    sdp: v8::Local<'scope, v8::String>,
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCPeerConnection, enumerable)]
struct RtcPeerConnectionPrototypeDeclaration {
    #[webapi(accessor_property, getter = rtc_peer_connection_state_getter, data = callback_data_index_value(scope, 0))]
    signaling_state: (),
    #[webapi(accessor_property = "iceGatheringState", getter = rtc_peer_connection_state_getter, data = callback_data_index_value(scope, 1))]
    ice_gathering_state: (),
    #[webapi(accessor_property = "iceConnectionState", getter = rtc_peer_connection_state_getter, data = callback_data_index_value(scope, 2))]
    ice_connection_state: (),
    #[webapi(accessor_property = "connectionState", getter = rtc_peer_connection_state_getter, data = callback_data_index_value(scope, 3))]
    connection_state: (),

    #[webapi(accessor_property = "localDescription", getter = rtc_peer_connection_description_getter, data = callback_data_index_value(scope, 0))]
    local_description: (),
    #[webapi(accessor_property = "currentLocalDescription", getter = rtc_peer_connection_description_getter, data = callback_data_index_value(scope, 1))]
    current_local_description: (),
    #[webapi(accessor_property = "pendingLocalDescription", getter = rtc_peer_connection_description_getter, data = callback_data_index_value(scope, 2))]
    pending_local_description: (),

    #[webapi(method = "createDataChannel", length = 1, callback = rtc_peer_connection_create_data_channel_callback)]
    create_data_channel: (),
    #[webapi(method = "createOffer", length = 0, callback = rtc_peer_connection_create_offer_callback)]
    create_offer: (),
    #[webapi(method = "setLocalDescription", length = 0, callback = rtc_peer_connection_set_local_description_callback)]
    set_local_description: (),
    #[webapi(method = "getConfiguration", length = 0, callback = rtc_peer_connection_get_configuration_callback)]
    get_configuration: (),
    #[webapi(method, length = 0, callback = rtc_peer_connection_close_callback)]
    close: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCRtpReceiver)]
struct RtcRtpReceiverConstructorDeclaration {
    #[webapi(static_method = "getCapabilities", enumerable, length = 1, callback = rtc_rtp_receiver_get_capabilities_callback)]
    get_capabilities: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCDataChannel, enumerable)]
struct RtcDataChannelPrototypeDeclaration {
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 0))]
    label: (),
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 1))]
    ordered: (),
    #[webapi(accessor_property = "maxPacketLifeTime", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 2))]
    max_packet_lifetime: (),
    #[webapi(accessor_property = "maxRetransmits", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 3))]
    max_retransmits: (),
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 4))]
    protocol: (),
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 5))]
    negotiated: (),
    #[webapi(accessor_property, getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 6))]
    id: (),
    #[webapi(accessor_property = "readyState", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 7))]
    ready_state: (),
    #[webapi(accessor_property = "bufferedAmount", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 8))]
    buffered_amount: (),
    #[webapi(accessor_property = "binaryType", getter = rtc_data_channel_value_getter, data = callback_data_index_value(scope, 9))]
    binary_type: (),
    #[webapi(method, length = 0, callback = rtc_data_channel_close_callback)]
    close: (),
    #[webapi(method, length = 1, callback = rtc_data_channel_send_callback)]
    send: (),
}

pub(in crate::context_bootstrap) fn install_webrtc_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "RTCIceCandidate" => {
            ice_candidate::install_ice_candidate_template_bindings(scope, template)
        }
        "RTCSessionDescription" => {
            session_description::install_session_description_template_bindings(scope, template)
        }
        "RTCPeerConnection" => {
            RtcPeerConnectionPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "RTCRtpReceiver" => {
            RtcRtpReceiverConstructorDeclaration::initialize_template(scope, template);
        }
        "RTCDataChannel" => {
            RtcDataChannelPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn rtc_peer_connection_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'RTCPeerConnection': Please use the 'new' operator.",
        );
        return;
    }
    let configuration = if args.length() == 0 || args.get(0).is_null_or_undefined() {
        v8::Object::new(scope)
    } else if let Ok(configuration) = v8::Local::<v8::Object>::try_from(args.get(0)) {
        configuration
    } else {
        throw_type_error(
            scope,
            "Failed to construct 'RTCPeerConnection': The provided configuration is not a dictionary.",
        );
        return;
    };
    let declaration = RtcPeerConnectionObjectDeclaration {
        configuration,
        signaling_state: v8str(scope, "stable"),
        ice_gathering_state: v8str(scope, "new"),
        ice_connection_state: v8str(scope, "new"),
        connection_state: v8str(scope, "new"),
        local_description: (),
        current_local_description: (),
        pending_local_description: (),
        has_data_channel: (),
        event_target_slot: (),
        ordered_handlers: (),
    };
    if declaration.initialize(scope, args.this()).is_err() {
        rv.set_undefined();
        return;
    }
    rv.set(args.this().into());
}

fn rtc_peer_connection_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_peer_connection_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        RTC_PEER_CONNECTION_STATE_SLOTS,
        "RTCPeerConnection state slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn rtc_peer_connection_description_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_peer_connection_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        RTC_PEER_CONNECTION_DESCRIPTION_SLOTS,
        "RTCPeerConnection description slots",
    ) else {
        rv.set_null();
        return;
    };
    rv.set(get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::null(scope).into()));
}

fn rtc_peer_connection_create_data_channel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_peer_connection_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'createDataChannel' on 'RTCPeerConnection': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(label) = args.get(0).to_string(scope) else {
        return;
    };
    let Some(channel) = build_rtc_data_channel(scope, label) else {
        rv.set_undefined();
        return;
    };
    set_private_value(
        scope,
        args.this(),
        RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    rv.set(channel.into());
}

fn rtc_peer_connection_create_offer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_peer_connection_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let options = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
    let audio = options
        .and_then(|options| options.get(scope, v8str(scope, "offerToReceiveAudio").into()))
        .is_some_and(|value| value.boolean_value(scope));
    let video = options
        .and_then(|options| options.get(scope, v8str(scope, "offerToReceiveVideo").into()))
        .is_some_and(|value| value.boolean_value(scope));
    let data = get_private_value(
        scope,
        args.this(),
        RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT,
    )
    .is_some_and(|value| value.boolean_value(scope));
    let sdp = build_signaling_only_offer(audio, video, data);
    let Some(sdp) = v8_string(scope, &sdp) else {
        rv.set_undefined();
        return;
    };
    let offer = RtcSessionDescriptionInitDeclaration::new(v8str(scope, "offer"), sdp)
        .bind(scope)
        .expect("RTC offer declaration should bind");
    set_resolved_promise(scope, &mut rv, offer.into());
}

fn rtc_peer_connection_set_local_description_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_peer_connection_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Ok(description) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        let reason = v8::Exception::type_error(
            scope,
            v8str(
                scope,
                "Failed to execute 'setLocalDescription' on 'RTCPeerConnection': The provided value is not a session description.",
            ),
        );
        set_rejected_promise(scope, &mut rv, reason);
        return;
    };
    set_private_value(
        scope,
        args.this(),
        RTC_PEER_CONNECTION_LOCAL_DESCRIPTION_SLOT,
        description.into(),
    );
    set_private_value(
        scope,
        args.this(),
        RTC_PEER_CONNECTION_PENDING_LOCAL_DESCRIPTION_SLOT,
        description.into(),
    );
    set_string_slot(
        scope,
        args.this(),
        RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT,
        "have-local-offer",
    );
    // Moli has no UDP/STUN/DTLS transport. Keep the ICE gatherer in its
    // initial state and publish no candidate events rather than fabricating
    // host/server-reflexive addresses or synchronously completing an
    // operation that Chromium starts on a later networking task.
    set_resolved_promise(scope, &mut rv, v8::undefined(scope).into());
}

fn rtc_peer_connection_get_configuration_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_peer_connection_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), RTC_PEER_CONNECTION_CONFIGURATION_SLOT)
            .unwrap_or_else(|| v8::Object::new(scope).into()),
    );
}

fn rtc_peer_connection_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_peer_connection_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    for (slot, state) in [
        (RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT, "closed"),
        (RTC_PEER_CONNECTION_ICE_CONNECTION_STATE_SLOT, "closed"),
        (RTC_PEER_CONNECTION_CONNECTION_STATE_SLOT, "closed"),
    ] {
        set_string_slot(scope, args.this(), slot, state);
    }
    rv.set_undefined();
}

fn rtc_rtp_receiver_get_capabilities_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(kind) = args.get(0).to_string(scope) else {
        rv.set_null();
        return;
    };
    let source = match kind.to_rust_string_lossy(scope).as_str() {
        "audio" => RTC_AUDIO_CAPABILITIES_JSON,
        "video" => RTC_VIDEO_CAPABILITIES_JSON,
        _ => {
            rv.set_null();
            return;
        }
    };
    let value =
        v8::json::parse(scope, v8str(scope, source)).unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

fn rtc_data_channel_value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_data_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(slot) = callback_data_item(
        scope,
        &args,
        RTC_DATA_CHANNEL_VALUE_SLOTS,
        "RTCDataChannel value slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn rtc_data_channel_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_data_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    set_string_slot(
        scope,
        args.this(),
        RTC_DATA_CHANNEL_READY_STATE_SLOT,
        "closed",
    );
    rv.set_undefined();
}

fn rtc_data_channel_send_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !rtc_data_channel_receiver_branded(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    rv.set_undefined();
}

fn build_rtc_data_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    label: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "RTCDataChannel")?;
    let channel = v8::Object::new(scope);
    if channel.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    RtcDataChannelObjectDeclaration::new(label)
        .initialize(scope, channel)
        .ok()?;
    Some(channel)
}

fn set_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: &'static str,
) {
    set_private_value(scope, object, slot, v8str(scope, value).into());
}

fn set_resolved_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.resolve(scope, value);
    rv.set(promise.into());
}

fn set_rejected_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.reject(scope, reason);
    rv.set(promise.into());
}

fn rtc_peer_connection_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::RTCPeerConnection::is_instance(scope, receiver)
}

fn rtc_data_channel_receiver_branded<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    web_api_interfaces::RTCDataChannel::is_instance(scope, receiver)
}

fn build_signaling_only_offer(audio: bool, video: bool, data: bool) -> String {
    let mut mids = Vec::new();
    if audio {
        mids.push("0");
    }
    if video {
        mids.push("1");
    }
    if data {
        mids.push("2");
    }
    let mut sdp = format!(
        "v=0\r\no=- 0 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE {}\r\na=extmap-allow-mixed\r\na=msid-semantic: WMS\r\n",
        mids.join(" ")
    );
    if audio {
        sdp.push_str(RTC_AUDIO_OFFER_SECTION);
    }
    if video {
        sdp.push_str(RTC_VIDEO_OFFER_SECTION);
    }
    if data {
        sdp.push_str(RTC_DATA_OFFER_SECTION);
    }
    sdp
}

const RTC_AUDIO_OFFER_SECTION: &str = concat!(
    "m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126\r\n",
    "c=IN IP4 0.0.0.0\r\na=mid:0\r\na=recvonly\r\na=rtcp-mux\r\na=rtcp-rsize\r\n",
    "a=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=1\r\n",
    "a=rtpmap:63 red/48000/2\r\na=fmtp:63 111/111\r\n",
    "a=rtpmap:9 G722/8000\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\n",
    "a=rtpmap:13 CN/8000\r\na=rtpmap:110 telephone-event/48000\r\n",
    "a=rtpmap:126 telephone-event/8000\r\n"
);

const RTC_VIDEO_OFFER_SECTION: &str = concat!(
    "m=video 9 UDP/TLS/RTP/SAVPF 96 97 98 99 100 101 35 36 37 38 103 104 107 108 109 114 115 116 117 118 39 40 41 42 43 44 45 46 47 48 119 120 121 49\r\n",
    "c=IN IP4 0.0.0.0\r\na=mid:1\r\na=recvonly\r\na=rtcp-mux\r\na=rtcp-rsize\r\n",
    "a=rtpmap:96 VP8/90000\r\na=rtpmap:97 rtx/90000\r\na=fmtp:97 apt=96\r\n",
    "a=rtpmap:98 VP9/90000\r\na=fmtp:98 profile-id=0\r\n",
    "a=rtpmap:99 rtx/90000\r\na=fmtp:99 apt=98\r\n",
    "a=rtpmap:100 VP9/90000\r\na=fmtp:100 profile-id=2\r\n",
    "a=rtpmap:101 rtx/90000\r\na=fmtp:101 apt=100\r\n",
    "a=rtpmap:35 VP9/90000\r\na=fmtp:35 profile-id=1\r\n",
    "a=rtpmap:36 rtx/90000\r\na=fmtp:36 apt=35\r\n",
    "a=rtpmap:37 VP9/90000\r\na=fmtp:37 profile-id=3\r\n",
    "a=rtpmap:38 rtx/90000\r\na=fmtp:38 apt=37\r\n",
    "a=rtpmap:103 H264/90000\r\na=fmtp:103 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f\r\n",
    "a=rtpmap:104 rtx/90000\r\na=fmtp:104 apt=103\r\n",
    "a=rtpmap:107 H264/90000\r\na=fmtp:107 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f\r\n",
    "a=rtpmap:108 rtx/90000\r\na=fmtp:108 apt=107\r\n",
    "a=rtpmap:109 H264/90000\r\na=fmtp:109 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n",
    "a=rtpmap:114 rtx/90000\r\na=fmtp:114 apt=109\r\n",
    "a=rtpmap:115 H264/90000\r\na=fmtp:115 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f\r\n",
    "a=rtpmap:116 rtx/90000\r\na=fmtp:116 apt=115\r\n",
    "a=rtpmap:117 H264/90000\r\na=fmtp:117 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d001f\r\n",
    "a=rtpmap:118 rtx/90000\r\na=fmtp:118 apt=117\r\n",
    "a=rtpmap:39 H264/90000\r\na=fmtp:39 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=4d001f\r\n",
    "a=rtpmap:40 rtx/90000\r\na=fmtp:40 apt=39\r\n",
    "a=rtpmap:41 H264/90000\r\na=fmtp:41 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=f4001f\r\n",
    "a=rtpmap:42 rtx/90000\r\na=fmtp:42 apt=41\r\n",
    "a=rtpmap:43 H264/90000\r\na=fmtp:43 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=f4001f\r\n",
    "a=rtpmap:44 rtx/90000\r\na=fmtp:44 apt=43\r\n",
    "a=rtpmap:45 AV1/90000\r\na=fmtp:45 level-idx=5;profile=0;tier=0\r\n",
    "a=rtpmap:46 rtx/90000\r\na=fmtp:46 apt=45\r\n",
    "a=rtpmap:47 AV1/90000\r\na=fmtp:47 level-idx=5;profile=1;tier=0\r\n",
    "a=rtpmap:48 rtx/90000\r\na=fmtp:48 apt=47\r\n",
    "a=rtpmap:119 red/90000\r\na=rtpmap:120 rtx/90000\r\na=fmtp:120 apt=119\r\n",
    "a=rtpmap:121 ulpfec/90000\r\na=rtpmap:49 flexfec-03/90000\r\n",
    "a=fmtp:49 repair-window=10000000\r\n"
);

const RTC_DATA_OFFER_SECTION: &str = concat!(
    "m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n",
    "c=IN IP4 0.0.0.0\r\na=mid:2\r\na=sctp-port:5000\r\na=max-message-size:262144\r\n"
);

const RTC_AUDIO_CAPABILITIES_JSON: &str = r#"{
  "codecs": [
    {"mimeType":"audio/opus","clockRate":48000,"channels":2,"sdpFmtpLine":"minptime=10;useinbandfec=1"},
    {"mimeType":"audio/red","clockRate":48000,"channels":2},
    {"mimeType":"audio/G722","clockRate":8000,"channels":1},
    {"mimeType":"audio/PCMU","clockRate":8000,"channels":1},
    {"mimeType":"audio/PCMA","clockRate":8000,"channels":1},
    {"mimeType":"audio/CN","clockRate":8000,"channels":1},
    {"mimeType":"audio/telephone-event","clockRate":48000,"channels":1},
    {"mimeType":"audio/telephone-event","clockRate":8000,"channels":1}
  ],
  "headerExtensions": [
    {"uri":"urn:ietf:params:rtp-hdrext:ssrc-audio-level"},
    {"uri":"http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time"},
    {"uri":"http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"},
    {"uri":"urn:ietf:params:rtp-hdrext:sdes:mid"}
  ]
}"#;

const RTC_VIDEO_CAPABILITIES_JSON: &str = r#"{
  "codecs": [
    {"mimeType":"video/VP8","clockRate":90000},
    {"mimeType":"video/rtx","clockRate":90000},
    {"mimeType":"video/VP9","clockRate":90000,"sdpFmtpLine":"profile-id=0"},
    {"mimeType":"video/VP9","clockRate":90000,"sdpFmtpLine":"profile-id=2"},
    {"mimeType":"video/VP9","clockRate":90000,"sdpFmtpLine":"profile-id=1"},
    {"mimeType":"video/VP9","clockRate":90000,"sdpFmtpLine":"profile-id=3"},
    {"mimeType":"video/H264","clockRate":90000,"sdpFmtpLine":"level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f"},
    {"mimeType":"video/H264","clockRate":90000,"sdpFmtpLine":"level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f"},
    {"mimeType":"video/H264","clockRate":90000,"sdpFmtpLine":"level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"},
    {"mimeType":"video/H264","clockRate":90000,"sdpFmtpLine":"level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f"},
    {"mimeType":"video/H264","clockRate":90000,"sdpFmtpLine":"level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d001f"},
    {"mimeType":"video/H264","clockRate":90000,"sdpFmtpLine":"level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=4d001f"},
    {"mimeType":"video/H264","clockRate":90000,"sdpFmtpLine":"level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=f4001f"},
    {"mimeType":"video/H264","clockRate":90000,"sdpFmtpLine":"level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=f4001f"},
    {"mimeType":"video/AV1","clockRate":90000,"sdpFmtpLine":"level-idx=5;profile=0;tier=0"},
    {"mimeType":"video/AV1","clockRate":90000,"sdpFmtpLine":"level-idx=5;profile=1;tier=0"},
    {"mimeType":"video/red","clockRate":90000},
    {"mimeType":"video/ulpfec","clockRate":90000},
    {"mimeType":"video/flexfec-03","clockRate":90000,"sdpFmtpLine":"repair-window=10000000"}
  ],
  "headerExtensions": [
    {"uri":"urn:ietf:params:rtp-hdrext:toffset"},
    {"uri":"http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time"},
    {"uri":"urn:3gpp:video-orientation"},
    {"uri":"http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"},
    {"uri":"http://www.webrtc.org/experiments/rtp-hdrext/playout-delay"},
    {"uri":"http://www.webrtc.org/experiments/rtp-hdrext/video-content-type"},
    {"uri":"http://www.webrtc.org/experiments/rtp-hdrext/video-timing"},
    {"uri":"http://www.webrtc.org/experiments/rtp-hdrext/color-space"},
    {"uri":"urn:ietf:params:rtp-hdrext:sdes:mid"},
    {"uri":"urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id"},
    {"uri":"urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id"}
  ]
}"#;
