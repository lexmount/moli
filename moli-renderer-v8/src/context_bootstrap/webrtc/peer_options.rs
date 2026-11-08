use super::*;
use crate::webidl;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCOfferAnswerOptions")]
pub(super) struct OfferAnswerOptions {
    #[webidl(default = true)]
    #[webidl(name = "voiceActivityDetection")]
    _voice_activity_detection: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCOfferOptions")]
pub(super) struct OfferOptions {
    #[webidl(inherit)]
    _base: OfferAnswerOptions,
    #[webidl(default = false)]
    #[webidl(name = "iceRestart")]
    _ice_restart: bool,
    pub(super) offer_to_receive_audio: Option<i32>,
    pub(super) offer_to_receive_video: Option<i32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.createOffer")]
pub(super) struct OfferArgs {
    #[webidl(dictionary)]
    pub(super) options: OfferOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.setLocalDescription")]
pub(super) struct LocalDescriptionArgs<'s> {
    #[webidl(dictionary)]
    pub(super) description: session_description::SessionDescriptionInit<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCDataChannelInit", snapshot)]
pub(super) struct DataChannelInit {
    #[webidl(converter = "enforce_range_unsigned_short", snapshot_slot = RTC_DATA_CHANNEL_ID_SLOT, snapshot_default = v8::null(scope).into())]
    pub(super) id: Option<u16>,
    #[webidl(name = "maxPacketLifeTime", snapshot_slot = RTC_DATA_CHANNEL_MAX_PACKET_LIFETIME_SLOT, snapshot_default = v8::null(scope).into())]
    pub(super) max_packet_lifetime: Option<u16>,
    #[webidl(snapshot_slot = RTC_DATA_CHANNEL_MAX_RETRANSMITS_SLOT, snapshot_default = v8::null(scope).into())]
    pub(super) max_retransmits: Option<u16>,
    #[webidl(default = false, snapshot_slot = RTC_DATA_CHANNEL_NEGOTIATED_SLOT)]
    pub(super) negotiated: bool,
    #[webidl(default = true, snapshot_slot = RTC_DATA_CHANNEL_ORDERED_SLOT)]
    ordered: bool,
    #[webidl(default = "", converter = "usv_string", snapshot_slot = RTC_DATA_CHANNEL_PROTOCOL_SLOT)]
    pub(super) protocol: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.createDataChannel")]
pub(super) struct DataChannelArgs {
    #[webidl(required, converter = "usv_string")]
    pub(super) label: String,
    #[webidl(dictionary)]
    pub(super) options: DataChannelInit,
}
