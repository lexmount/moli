use crate::parse::{mime_essence, mime_parameter};

mod codecs;
use codecs::Codec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaTrackKind {
    Audio,
    Video,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaMimeSupport {
    Unsupported,
    Maybe,
    Probably,
}

impl MediaMimeSupport {
    pub fn as_can_play_type(self) -> &'static str {
        match self {
            Self::Unsupported => "",
            Self::Maybe => "maybe",
            Self::Probably => "probably",
        }
    }
}

pub fn media_mime_support(input: &str) -> MediaMimeSupport {
    let Some(mime) = mime_essence(input) else {
        return MediaMimeSupport::Unsupported;
    };
    if let Some(codecs) = mime_parameter(input, "codecs") {
        return if codecs
            .split(',')
            .all(|label| Codec::parse(label).is_some_and(|codec| codec.allowed_in(&mime)))
        {
            MediaMimeSupport::Probably
        } else {
            MediaMimeSupport::Unsupported
        };
    }
    match mime.as_str() {
        "audio/mp3" | "audio/x-mp3" | "audio/mpeg" | "audio/aac" | "audio/flac" => {
            MediaMimeSupport::Probably
        }
        "audio/webm" | "audio/ogg" | "audio/wav" | "audio/x-wav" | "audio/mp4" | "video/mp4"
        | "video/webm" | "video/ogg" | "application/ogg" => MediaMimeSupport::Maybe,
        _ => MediaMimeSupport::Unsupported,
    }
}

/// Unlike canPlayType, a decoding configuration describes a single track and
/// cannot use an ambiguous container-only or multi-codec declaration.
pub fn is_media_decoding_type_supported(input: &str, kind: MediaTrackKind) -> bool {
    if media_mime_support(input) != MediaMimeSupport::Probably {
        return false;
    }
    if let Some(label) = mime_parameter(input, "codecs") {
        return Codec::parse(&label)
            .is_some_and(|codec| codec.is_audio() == (kind == MediaTrackKind::Audio));
    }
    kind == MediaTrackKind::Audio
}

pub fn is_media_source_type_supported(input: &str) -> bool {
    matches!(
        mime_essence(input).as_deref(),
        Some("audio/mpeg" | "audio/aac" | "audio/mp4" | "video/mp4" | "audio/webm" | "video/webm")
    ) && media_mime_support(input) == MediaMimeSupport::Probably
}
