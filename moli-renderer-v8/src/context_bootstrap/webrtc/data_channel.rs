//! Data-channel initialization and native attributes. SCTP establishment and
//! message transport are not implemented by the signaling frontend.

use super::*;

pub(super) const CHANNELS: &str = "__moliRtcDataChannels";
const LABEL: &str = "__moliRtcDataChannelLabel";
const ORDERED: &str = "__moliRtcDataChannelOrdered";
const MAX_PACKET_LIFETIME: &str = "__moliRtcDataChannelMaxPacketLifetime";
const MAX_RETRANSMITS: &str = "__moliRtcDataChannelMaxRetransmits";
const PROTOCOL: &str = "__moliRtcDataChannelProtocol";
const NEGOTIATED: &str = "__moliRtcDataChannelNegotiated";
const ID: &str = "__moliRtcDataChannelId";
const READY_STATE: &str = "__moliRtcDataChannelReadyState";
const BUFFERED_AMOUNT: &str = "__moliRtcDataChannelBufferedAmount";
const THRESHOLD: &str = "__moliRtcDataChannelBufferedAmountLowThreshold";
const BINARY_TYPE: &str = "__moliRtcDataChannelBinaryType";
const LISTENERS: &str = "__moliRtcDataChannelListeners";
const VALUES: &[&str] = &[
    LABEL,
    ORDERED,
    MAX_PACKET_LIFETIME,
    MAX_RETRANSMITS,
    PROTOCOL,
    NEGOTIATED,
    ID,
    READY_STATE,
    BUFFERED_AMOUNT,
    THRESHOLD,
    BINARY_TYPE,
];

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCDataChannelInit")]
struct Init {
    #[webidl(default = true)]
    ordered: bool,
    #[webidl(converter = "enforce_range_unsigned_short", name = "maxPacketLifeTime")]
    max_packet_lifetime: Option<u16>,
    #[webidl(converter = "enforce_range_unsigned_short")]
    max_retransmits: Option<u16>,
    #[webidl(default = "", converter = "usv_string")]
    protocol: String,
    #[webidl(default = false)]
    negotiated: bool,
    #[webidl(converter = "enforce_range_unsigned_short")]
    id: Option<u16>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.createDataChannel")]
struct CreateArgs {
    #[webidl(required, converter = "usv_string")]
    label: String,
    #[webidl(dictionary)]
    options: Init,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCDataChannel, require_prototype)]
struct Channel<'s> {
    #[webapi(slot = LABEL)]
    label: v8::Local<'s, v8::String>,
    #[webapi(slot = ORDERED)]
    ordered: bool,
    #[webapi(slot = MAX_PACKET_LIFETIME)]
    max_packet_lifetime: v8::Local<'s, v8::Value>,
    #[webapi(slot = MAX_RETRANSMITS)]
    max_retransmits: v8::Local<'s, v8::Value>,
    #[webapi(slot = PROTOCOL)]
    protocol: v8::Local<'s, v8::String>,
    #[webapi(slot = NEGOTIATED)]
    negotiated: bool,
    #[webapi(slot = ID)]
    id: v8::Local<'s, v8::Value>,
    #[webapi(slot = READY_STATE, init = string("connecting"))]
    ready_state: (),
    #[webapi(slot = BUFFERED_AMOUNT, init = 0)]
    buffered_amount: (),
    #[webapi(slot = THRESHOLD, init = 0)]
    threshold: (),
    #[webapi(slot = BINARY_TYPE, init = string("arraybuffer"))]
    binary_type: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS)]
    event_target_slot: (),
    #[webapi(slot = SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCDataChannel, enumerable, receiver)]
struct Prototype {
    #[webapi(accessor_property, getter = value, data = callback_data_index_value(scope, 0))]
    label: (),
    #[webapi(accessor_property, getter = value, data = callback_data_index_value(scope, 1))]
    ordered: (),
    #[webapi(accessor_property = "maxPacketLifeTime", getter = value, data = callback_data_index_value(scope, 2))]
    max_packet_lifetime: (),
    #[webapi(accessor_property, getter = value, data = callback_data_index_value(scope, 3))]
    max_retransmits: (),
    #[webapi(accessor_property, getter = value, data = callback_data_index_value(scope, 4))]
    protocol: (),
    #[webapi(accessor_property, getter = value, data = callback_data_index_value(scope, 5))]
    negotiated: (),
    #[webapi(accessor_property, getter = value, data = callback_data_index_value(scope, 6))]
    id: (),
    #[webapi(accessor_property, getter = value, data = callback_data_index_value(scope, 7))]
    ready_state: (),
    #[webapi(accessor_property, getter = value, data = callback_data_index_value(scope, 8))]
    buffered_amount: (),
    #[webapi(accessor_property, getter = value, setter = set_threshold, data = callback_data_index_value(scope, 9))]
    buffered_amount_low_threshold: (),
    #[webapi(accessor_property, getter = value, setter = set_binary_type, data = callback_data_index_value(scope, 10))]
    binary_type: (),
    #[webapi(method, length = 0, callback = close)]
    close: (),
    #[webapi(method, length = 1, callback = send)]
    send: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    Prototype::initialize_prototype_template(scope, prototype);
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(CreateArgs { label, options }) = webidl::parse_args(scope, &args) else {
        return;
    };
    let pc = rtp_transceivers::target(scope, args.this());
    if rtp_transceivers::closed(scope, pc) {
        crate::native_bridge::throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The connection is closed.",
        );
        return;
    }
    // WebIDL converts every argument first. Algorithm validation cannot skip
    // dictionary getters or their exceptions, even on a closed connection.
    let id = options.id.filter(|_| options.negotiated);
    if label.len() > usize::from(u16::MAX)
        || options.protocol.len() > usize::from(u16::MAX)
        || (options.negotiated && id.is_none())
        || id == Some(u16::MAX)
        || (options.max_packet_lifetime.is_some() && options.max_retransmits.is_some())
    {
        throw_type_error(scope, "Invalid RTCDataChannel initialization.");
        return;
    }
    let channels = v8::Local::<v8::Array>::try_from(
        get_private_value(scope, pc, CHANNELS).expect("peer data channels"),
    )
    .expect("peer data channel list");
    if let Some(id) = id {
        for index in 0..channels.length() {
            let channel = v8::Local::<v8::Object>::try_from(
                channels.get_index(scope, index).expect("channel"),
            )
            .expect("channel object");
            if get_private_value(scope, channel, ID)
                .expect("channel id")
                .strict_equals(v8::Integer::new_from_unsigned(scope, u32::from(id)).into())
                && !get_private_value(scope, channel, READY_STATE)
                    .expect("channel state")
                    .strict_equals(v8str(scope, "closed").into())
            {
                crate::native_bridge::throw_dom_exception(
                    scope,
                    "OperationError",
                    0,
                    "The data channel id is already in use.",
                );
                return;
            }
        }
    }
    let Some(label) = v8_string(scope, &label) else {
        return;
    };
    let Some(protocol) = v8_string(scope, &options.protocol) else {
        return;
    };
    let declaration = Channel {
        label,
        ordered: options.ordered,
        max_packet_lifetime: nullable_unsigned_short(scope, options.max_packet_lifetime),
        max_retransmits: nullable_unsigned_short(scope, options.max_retransmits),
        protocol,
        negotiated: options.negotiated,
        id: nullable_unsigned_short(scope, id),
        ready_state: (),
        buffered_amount: (),
        threshold: (),
        binary_type: (),
        event_target_slot: (),
        ordered_handlers: (),
    };
    let Ok(channel) = declaration.bind(scope) else {
        return;
    };
    if moli_webapi_declare::define_array_data_property(
        scope,
        channels,
        channels.length(),
        channel.into(),
    )
    .is_none()
    {
        return;
    }
    let first = channels.length() == 1;
    set_private_value(
        scope,
        pc,
        RTC_PEER_CONNECTION_HAS_DATA_CHANNEL_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    if first {
        rtp_transceivers::update_negotiation_needed(scope, pc);
    }
    rv.set(channel.into());
}

fn nullable_unsigned_short<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<u16>,
) -> v8::Local<'s, v8::Value> {
    value
        .map(|value| v8::Integer::new_from_unsigned(scope, u32::from(value)).into())
        .unwrap_or_else(|| v8::null(scope).into())
}

fn value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, VALUES, "RTCDataChannel attribute") else {
        return;
    };
    let channel = rtp_transceivers::target(scope, args.this());
    rv.set(get_private_value(scope, channel, slot).expect("data channel slot"));
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCDataChannel.bufferedAmountLowThreshold")]
struct ThresholdArgs {
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    value: u32,
}

fn set_threshold<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(ThresholdArgs { value }) = webidl::parse_args(scope, &args) else {
        return;
    };
    let channel = rtp_transceivers::target(scope, args.this());
    set_private_value(
        scope,
        channel,
        THRESHOLD,
        v8::Integer::new_from_unsigned(scope, value).into(),
    );
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCDataChannel.binaryType")]
struct BinaryTypeArgs {
    #[webidl(default = "undefined")]
    value: String,
}

fn set_binary_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(BinaryTypeArgs { value }) = webidl::parse_args(scope, &args) else {
        return;
    };
    // WebIDL silently ignores out-of-enum values for an attribute setter,
    // after performing observable DOMString conversion.
    if matches!(value.as_str(), "blob" | "arraybuffer") {
        let channel = rtp_transceivers::target(scope, args.this());
        set_private_value(
            scope,
            channel,
            BINARY_TYPE,
            v8_string(scope, &value).expect("binary type").into(),
        );
    }
}

fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // The existing local close transition is retained until SCTP closing tasks
    // and peer-connection shutdown are implemented together.
    let channel = rtp_transceivers::target(scope, args.this());
    set_string_slot(scope, channel, READY_STATE, "closed");
    rv.set_undefined();
}

fn send<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Existing shell; sending, state errors and buffered accounting are pending.
    rv.set_undefined();
}
