use crate::util::v8str;
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCRtpSender)]
struct RtcRtpSenderConstructorDeclaration {
    #[webapi(static_method = "getCapabilities", enumerable, length = 1, callback = get_capabilities)]
    get_capabilities: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCRtpReceiver)]
struct RtcRtpReceiverConstructorDeclaration {
    #[webapi(static_method = "getCapabilities", enumerable, length = 1, callback = get_capabilities)]
    get_capabilities: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "getCapabilities")]
struct GetCapabilitiesArgs {
    #[webidl(required)]
    kind: String,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "RTCRtpSender" => RtcRtpSenderConstructorDeclaration::initialize_template(scope, template),
        "RTCRtpReceiver" => {
            RtcRtpReceiverConstructorDeclaration::initialize_template(scope, template)
        }
        _ => {}
    }
}

fn get_capabilities<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<GetCapabilitiesArgs>(scope, &args) else {
        return;
    };
    let source = match parsed.kind.as_str() {
        "audio" => RTC_AUDIO_CAPABILITIES_JSON,
        "video" => RTC_VIDEO_CAPABILITIES_JSON,
        _ => {
            rv.set_null();
            return;
        }
    };
    // Intrinsic parsing creates fresh dictionaries in the callee realm without
    // consulting public JSON methods or invoking inherited property setters.
    if let Some(value) = v8::json::parse(scope, v8str(scope, source)) {
        rv.set(value);
    }
}

// Compatibility tables shared with the existing receiver surface. These do not
// represent a native codec negotiation or media encoding backend.
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
