//! Host-owned RTP objects and WebRTC tasks.

use moli_webapi::event_target::SIMPLE_EVENT_TARGET_LISTENERS_SLOT;
use moli_webapi::rtc::peer_connection::{
    RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT, rtp_capabilities,
    complete_create_offer, complete_set_local_description,
};

mod operations;
mod stats;
mod dtmf;
mod data_channel;
mod signaling;
mod rtp_sender;
mod rtp_offer;
mod sdp;
mod rtp_parameters;
pub(crate) mod rtp_transceivers;

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface: &str,
) {
    let prototype = template.prototype_template(scope);
    rtp_transceivers::install(scope, prototype, interface);
    if interface == "RTCPeerConnection" {
        signaling::install(scope, prototype);
        stats::install_peer(scope, prototype);
        data_channel::install_peer(scope, prototype);
    }
    if interface == "RTCDataChannel" {
        data_channel::install(scope, prototype);
    }
    if interface == "RTCStatsReport" {
        stats::install(scope, prototype);
    }
    if interface == "RTCDTMFSender" {
        dtmf::install(scope, prototype);
    }
}

pub(super) fn install_runtime(scope: &mut v8::PinScope<'_, '_>) {
    moli_webapi::rtc::peer_connection::signaling::install_runtime(
        scope,
        moli_webapi::rtc::peer_connection::signaling::SignalingRuntime {
            dispatch_state_change: signaling::dispatch_state_change,
        },
    );
    moli_webapi::rtc::peer_connection::operations::install_runtime(
        scope,
        moli_webapi::rtc::peer_connection::operations::OperationsRuntime {
            initialize: operations::initialize,
            enqueue: operations::enqueue,
            payload: operations::payload,
            resolve: operations::resolve,
            reject: operations::reject,
            close: operations::close,
        },
    );
    moli_webapi::rtc::peer_connection::rtp_transceivers::install_runtime(
        scope,
        moli_webapi::rtc::peer_connection::rtp_transceivers::RtpRuntime {
            initialize_pc,
            close: rtp_transceivers::close,
            create_offer,
        },
    );
}

fn create_offer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    data: bool,
) -> Option<String> {
    rtp_transceivers::has_transceivers(scope, pc).then(|| {
        let sections = rtp_transceivers::offer_sections(scope, pc);
        rtp_offer::build(&sections, data)
    })
}

fn initialize_pc<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> Option<()> {
    rtp_transceivers::initialize_pc(scope, pc);
    let channels = v8::Array::new(scope, 0);
    crate::util::set_private_value(scope, pc, data_channel::CHANNELS, channels.into());
    let id = super::media_streams::identifier(scope)?;
    crate::util::set_private_value(scope, pc, stats::ID, id.into());
    rtp_sender::initialize_pc(scope, pc)
}
