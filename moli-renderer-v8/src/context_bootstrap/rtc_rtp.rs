//! Host-owned RTP objects and WebRTC tasks.

use moli_webapi::rtc::peer_connection::{
    RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT, rtp_capabilities,
};

mod rtp_offer;
mod rtp_parameters;
pub(crate) mod rtp_transceivers;

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface: &str,
) {
    let prototype = template.prototype_template(scope);
    rtp_transceivers::install(scope, prototype, interface);
}

pub(super) fn install_runtime(scope: &mut v8::PinScope<'_, '_>) {
    moli_webapi::rtc::peer_connection::rtp_transceivers::install_runtime(
        scope,
        moli_webapi::rtc::peer_connection::rtp_transceivers::RtpRuntime {
            initialize_pc: rtp_transceivers::initialize_pc,
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
