//! RTC descriptions, candidates, and encoded frame values; transport stays in the embedder.

mod dictionary_string;
pub mod encoded_frames;
pub mod ice_candidate;
mod ice_candidate_parser;
pub mod peer_connection;
pub mod session_description;

pub use crate::events::ice_error_event::rtc_peer_connection_ice_error_event_constructor;
pub use crate::events::webrtc::{
    rtc_data_channel_event_constructor_callback, rtc_error_event_constructor_callback,
    rtc_peer_connection_ice_event_constructor_callback,
};
pub use crate::events::webrtc_payload::{
    rtc_dtmf_tone_event_constructor, rtc_track_event_constructor,
};
pub use ice_candidate::rtc_ice_candidate_constructor_callback;
pub use peer_connection::rtc_peer_connection_constructor_callback;
pub use session_description::rtc_session_description_constructor_callback;

pub fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface {
        "RTCEncodedAudioFrame" | "RTCEncodedVideoFrame" => {
            encoded_frames::install(scope, prototype, interface)
        }
        "RTCPeerConnectionIceErrorEvent" => {
            crate::events::ice_error_event::install(scope, prototype)
        }
        "RTCPeerConnectionIceEvent" | "RTCDataChannelEvent" | "RTCErrorEvent" => {
            crate::events::webrtc::install_event_template_bindings(scope, template, interface)
        }
        "RTCDTMFToneChangeEvent" | "RTCTrackEvent" => {
            crate::events::webrtc_payload::install(scope, prototype, interface)
        }
        "RTCIceCandidate" => {
            ice_candidate::install_ice_candidate_template_bindings(scope, template)
        }
        "RTCSessionDescription" => {
            session_description::install_session_description_template_bindings(scope, template)
        }
        "RTCPeerConnection" | "RTCDataChannel" | "RTCRtpSender" | "RTCRtpReceiver" => {
            peer_connection::install_template_bindings(scope, template, interface)
        }
        _ => {}
    }
}
