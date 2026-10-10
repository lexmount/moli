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

