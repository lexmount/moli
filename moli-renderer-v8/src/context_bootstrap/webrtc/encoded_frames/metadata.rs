use crate::webidl;
use moli_webapi_declare::{WebApiObject, WebApiValue};

struct Text(webidl::DomString16);

impl<'s> webidl::WebIdlConverter<'s> for Text {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::convert::<webidl::DomString16>(scope, value, context).map(Self)
    }
}

impl<'s> WebApiValue<'s> for Text {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        v8::String::new_from_two_byte(scope, &self.0.0, v8::NewStringType::Normal).map(Into::into)
    }
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "RTCEncodedFrameMetadata")]
#[webapi(plain, enumerable)]
struct BaseMetadata {
    #[webidl(converter = "double")]
    #[webapi(data_property)]
    capture_time: Option<f64>,
    #[webidl(sequence, converter = "unsigned_long")]
    #[webapi(data_property)]
    contributing_sources: Option<Vec<u32>>,
    #[webidl(converter = "raw")]
    #[webapi(data_property)]
    mime_type: Option<Text>,
    #[webidl(converter = "octet")]
    #[webapi(data_property)]
    payload_type: Option<u8>,
    #[webidl(converter = "double")]
    #[webapi(data_property)]
    receive_time: Option<f64>,
    #[webidl(converter = "unsigned_long")]
    #[webapi(data_property)]
    rtp_timestamp: Option<u32>,
    #[webidl(converter = "double")]
    #[webapi(data_property)]
    sender_capture_time_offset: Option<f64>,
    #[webidl(converter = "unsigned_long")]
    #[webapi(data_property)]
    synchronization_source: Option<u32>,
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "RTCEncodedAudioFrameMetadata")]
#[webapi(plain, enumerable)]
pub(super) struct AudioMetadata {
    #[webidl(inherit)]
    base: BaseMetadata,
    #[webidl(converter = "double")]
    #[webapi(data_property)]
    audio_level: Option<f64>,
    #[webidl(converter = "short")]
    #[webapi(data_property)]
    sequence_number: Option<i16>,
}

impl AudioMetadata {
    pub(super) fn object<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        let object = self.base.bind(scope).expect("native base frame metadata");
        self.initialize(scope, object)
            .expect("native audio frame metadata");
        object
    }
}

#[derive(webidl::WebIdlDictionary, WebApiObject)]
#[webidl(prefix = "RTCEncodedVideoFrameMetadata")]
#[webapi(plain, enumerable)]
pub(super) struct VideoMetadata {
    #[webidl(inherit)]
    base: BaseMetadata,
    #[webidl(sequence, converter = "unsigned_long_long")]
    #[webapi(data_property)]
    dependencies: Option<Vec<u64>>,
    #[webidl(converter = "unsigned_long_long")]
    #[webapi(data_property)]
    frame_id: Option<u64>,
    #[webidl(converter = "unsigned_short")]
    #[webapi(data_property)]
    height: Option<u16>,
    #[webidl(converter = "unsigned_long")]
    #[webapi(data_property)]
    spatial_index: Option<u32>,
    #[webidl(converter = "unsigned_long")]
    #[webapi(data_property)]
    temporal_index: Option<u32>,
    #[webidl(converter = "long_long")]
    #[webapi(data_property)]
    timestamp: Option<i64>,
    #[webidl(converter = "unsigned_short")]
    #[webapi(data_property)]
    width: Option<u16>,
}

impl VideoMetadata {
    pub(super) fn object<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        let object = self.base.bind(scope).expect("native base frame metadata");
        self.initialize(scope, object)
            .expect("native video frame metadata");
        object
    }
}

pub(super) const BASE_MEMBERS: &[&str] = &[
    "captureTime",
    "contributingSources",
    "mimeType",
    "payloadType",
    "receiveTime",
    "rtpTimestamp",
    "senderCaptureTimeOffset",
    "synchronizationSource",
];
pub(super) const AUDIO_MEMBERS: &[&str] = &["audioLevel", "sequenceNumber"];
pub(super) const VIDEO_MEMBERS: &[&str] = &[
    "dependencies",
    "frameId",
    "height",
    "spatialIndex",
    "temporalIndex",
    "timestamp",
    "width",
];
