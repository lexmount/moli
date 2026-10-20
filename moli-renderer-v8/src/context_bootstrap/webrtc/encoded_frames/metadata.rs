use crate::webidl;
use webidl::{SnapshotPrototype, WebIdlDictionarySerialize};

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCEncodedFrameMetadata", snapshot)]
struct BaseMetadata {
    #[webidl(converter = "double")]
    capture_time: Option<f64>,
    #[webidl(sequence, converter = "unsigned_long")]
    contributing_sources: Option<Vec<u32>>,
    #[webidl(converter = "raw")]
    mime_type: Option<webidl::DomString16>,
    #[webidl(converter = "octet")]
    payload_type: Option<u8>,
    #[webidl(converter = "double")]
    receive_time: Option<f64>,
    #[webidl(converter = "unsigned_long")]
    rtp_timestamp: Option<u32>,
    #[webidl(converter = "double")]
    sender_capture_time_offset: Option<f64>,
    #[webidl(converter = "unsigned_long")]
    synchronization_source: Option<u32>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCEncodedAudioFrameMetadata", snapshot)]
pub(super) struct AudioMetadata {
    #[webidl(inherit)]
    base: BaseMetadata,
    #[webidl(converter = "double")]
    audio_level: Option<f64>,
    #[webidl(converter = "short")]
    sequence_number: Option<i16>,
}

impl AudioMetadata {
    pub(super) fn object<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        self.to_js_snapshot(scope, SnapshotPrototype::Ordinary)
            .expect("native audio frame metadata")
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCEncodedVideoFrameMetadata", snapshot)]
pub(super) struct VideoMetadata {
    #[webidl(inherit)]
    base: BaseMetadata,
    #[webidl(sequence, converter = "unsigned_long_long")]
    dependencies: Option<Vec<u64>>,
    #[webidl(converter = "unsigned_long_long")]
    frame_id: Option<u64>,
    #[webidl(converter = "unsigned_short")]
    height: Option<u16>,
    #[webidl(converter = "unsigned_long")]
    spatial_index: Option<u32>,
    #[webidl(converter = "unsigned_long")]
    temporal_index: Option<u32>,
    #[webidl(converter = "long_long")]
    timestamp: Option<i64>,
    #[webidl(converter = "unsigned_short")]
    width: Option<u16>,
}

impl VideoMetadata {
    pub(super) fn object<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        self.to_js_snapshot(scope, SnapshotPrototype::Ordinary)
            .expect("native video frame metadata")
    }
}
