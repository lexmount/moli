use crate::parse::{mime_essence, mime_parameter};

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
    match mime.as_str() {
        "audio/mp3" | "audio/mpeg" | "audio/webm" | "audio/ogg" | "audio/wav" => {
            MediaMimeSupport::Probably
        }
        "audio/aac" | "audio/flac" => MediaMimeSupport::Maybe,
        "video/mp4" | "video/webm" | "video/ogg" => MediaMimeSupport::Probably,
        _ => MediaMimeSupport::Unsupported,
    }
}

pub fn is_media_source_type_supported(input: &str) -> bool {
    media_source_tracks(input, false).is_some()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaSourceTracks {
    Video,
    Audio,
    VideoAndAudio,
}

impl MediaSourceTracks {
    pub fn has_video(self) -> bool {
        matches!(self, Self::Video | Self::VideoAndAudio)
    }

    pub fn has_audio(self) -> bool {
        matches!(self, Self::Audio | Self::VideoAndAudio)
    }
}

pub fn media_source_tracks(input: &str, supports_aac: bool) -> Option<MediaSourceTracks> {
    let mime = mime_essence(input)?;
    let codecs = mime_parameter(input, "codecs")?;
    let mut video = false;
    let mut audio = false;
    for codec in codecs.split(',').map(str::trim) {
        if !video && codec.to_ascii_lowercase().starts_with("avc1.") {
            video = true;
        } else if supports_aac && !audio && codec.eq_ignore_ascii_case("mp4a.40.2") {
            audio = true;
        } else {
            return None;
        }
    }
    match (mime.as_str(), video, audio) {
        ("video/mp4", true, false) => Some(MediaSourceTracks::Video),
        ("audio/mp4", false, true) => Some(MediaSourceTracks::Audio),
        ("video/mp4", true, true) => Some(MediaSourceTracks::VideoAndAudio),
        _ => None,
    }
}
