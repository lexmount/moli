use crate::parse::parse_mime_type;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Audio,
    Video,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CodecParameters {
    Required,
    Implied,
    Optional,
}

/// Checks a MediaCapabilities track's MIME description independently of codec support.
/// Container formats describe one track with `codecs`; RTP codec types retain
/// their payload-format parameters instead. Unsupported MIME types can still
/// be valid descriptions.
pub fn is_valid_media_capabilities_mime_type(
    content_type: &str,
    kind: MediaKind,
    is_webrtc: bool,
) -> bool {
    if content_type.trim_end().ends_with(';') {
        return false;
    }
    let Some(mime) = parse_mime_type(content_type) else {
        return false;
    };
    let media_type = match kind {
        MediaKind::Audio => "audio",
        MediaKind::Video => "video",
    };
    if mime.type_() != media_type && mime.type_() != "application" {
        return false;
    }
    let rule = match mime.essence().as_str() {
        // RFC 6381, RFC 5334 and WebM container MIME types can carry several codecs.
        "audio/3gpp" | "video/3gpp" | "audio/3gpp2" | "video/3gpp2" | "audio/mp4" | "video/mp4"
        | "application/mp4" | "video/quicktime" | "application/mp21" | "audio/ogg"
        | "video/ogg" | "application/ogg" | "audio/webm" | "video/webm" => {
            CodecParameters::Required
        }
        "audio/aac" | "audio/flac" | "audio/mpeg" | "audio/mp3" | "audio/opus" | "audio/vorbis"
        | "audio/ac3" | "audio/eac3" | "video/vp8" | "video/vp9" | "video/h264" | "video/h265"
        | "video/av1" => CodecParameters::Implied,
        _ => CodecParameters::Optional,
    };
    if is_webrtc && rule != CodecParameters::Required {
        return true;
    }
    match mime.parameters() {
        [] => rule != CodecParameters::Required,
        [(name, codecs)] if name == "codecs" && rule != CodecParameters::Implied => {
            single_codec_matches_kind(codecs, kind)
        }
        _ => false,
    }
}

fn single_codec_matches_kind(codecs: &str, kind: MediaKind) -> bool {
    let codec = codecs.trim_matches(|value: char| value.is_ascii_whitespace());
    if codec.is_empty() || !codec.bytes().all(is_codec_token_byte) {
        return false;
    }
    let codec = codec.to_ascii_lowercase();
    let opposite = match kind {
        MediaKind::Audio => &[
            "av01", "avc1", "avc3", "hev1", "hvc1", "theora", "vp8", "vp9", "vp09", "h264", "h265",
        ][..],
        MediaKind::Video => &[
            "aac", "ac-3", "ec-3", "flac", "mp3", "mp4a", "opus", "vorbis", "samr", "sawb",
        ][..],
    };
    !opposite.iter().any(|family| {
        codec == *family
            || codec
                .strip_prefix(*family)
                .is_some_and(|suffix| suffix.starts_with('.') || suffix.starts_with('-'))
    })
}

fn is_codec_token_byte(value: u8) -> bool {
    // RFC 6381's unencoded codec identifier uses RFC 2045 TOKEN characters.
    (0x21..=0x7e).contains(&value)
        && !matches!(
            value,
            b'(' | b')'
                | b'<'
                | b'>'
                | b'@'
                | b','
                | b';'
                | b':'
                | b'\\'
                | b'"'
                | b'/'
                | b'['
                | b']'
                | b'?'
                | b'='
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_describe_exactly_one_codec_in_both_transports() {
        for (essence, kind, codec, opposite) in [
            ("audio/webm", MediaKind::Audio, "opus", "vp8"),
            ("video/webm", MediaKind::Video, "vp8", "opus"),
            ("audio/ogg", MediaKind::Audio, "vorbis", "theora"),
            ("video/ogg", MediaKind::Video, "theora", "vorbis"),
            ("application/ogg", MediaKind::Audio, "vorbis", "theora"),
            ("application/ogg", MediaKind::Video, "theora", "vorbis"),
            ("audio/mp4", MediaKind::Audio, "mp4a.40.2", "avc1.42e01e"),
            ("video/mp4", MediaKind::Video, "avc1.42e01e", "mp4a.40.2"),
            (
                "application/mp4",
                MediaKind::Video,
                "avc1.42e01e",
                "mp4a.40.2",
            ),
            ("audio/3gpp", MediaKind::Audio, "samr", "avc1.42e01e"),
            ("video/3gpp2", MediaKind::Video, "avc1.42e01e", "samr"),
            (
                "video/quicktime",
                MediaKind::Video,
                "avc1.42e01e",
                "mp4a.40.2",
            ),
            (
                "application/mp21",
                MediaKind::Video,
                "avc1.42e01e",
                "mp4a.40.2",
            ),
        ] {
            for is_webrtc in [false, true] {
                let valid = format!("{essence}; codecs=\"{codec}\"");
                assert!(is_valid_media_capabilities_mime_type(
                    &valid, kind, is_webrtc
                ));
                for invalid in [
                    essence.to_owned(),
                    format!("{valid}; foo=bar"),
                    format!("{essence}; foo=bar"),
                    format!("{essence}; codecs=\"{codec},{codec}\""),
                    format!("{essence}; codecs=\",{codec}\""),
                    format!("{essence}; codecs=\"{codec},\""),
                    format!("{essence}; codecs=\"\""),
                    format!("{essence}; codecs=\"{opposite}\""),
                ] {
                    assert!(
                        !is_valid_media_capabilities_mime_type(&invalid, kind, is_webrtc),
                        "{invalid}, WebRTC={is_webrtc}"
                    );
                }
            }
        }
    }

    #[test]
    fn rtp_parameters_are_distinct_from_implied_file_codecs() {
        for (essence, kind, parameters) in [
            ("audio/opus", MediaKind::Audio, "minptime=10;useinbandfec=1"),
            ("video/VP9", MediaKind::Video, "profile-id=0"),
            (
                "video/H264",
                MediaKind::Video,
                "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
            ),
            (
                "video/AV1",
                MediaKind::Video,
                "level-idx=5;profile=0;tier=0",
            ),
        ] {
            assert!(is_valid_media_capabilities_mime_type(essence, kind, false));
            let parameterized = format!("{essence}; {parameters}");
            assert!(is_valid_media_capabilities_mime_type(
                &parameterized,
                kind,
                true
            ));
            assert!(!is_valid_media_capabilities_mime_type(
                &parameterized,
                kind,
                false
            ));
        }
        for essence in ["audio/mpeg", "audio/mp3", "audio/aac", "audio/flac"] {
            assert!(is_valid_media_capabilities_mime_type(
                essence,
                MediaKind::Audio,
                false
            ));
            assert!(!is_valid_media_capabilities_mime_type(
                &format!("{essence}; codecs=opaque"),
                MediaKind::Audio,
                false
            ));
        }
    }

    #[test]
    fn mime_records_preserve_first_parameters_and_validate_codec_tokens() {
        for is_webrtc in [false, true] {
            assert!(is_valid_media_capabilities_mime_type(
                "Video/WebM; CODECS=vp8; codecs=opus",
                MediaKind::Video,
                is_webrtc
            ));
            assert!(is_valid_media_capabilities_mime_type(
                "video/x-future-media; codecs=future-codec",
                MediaKind::Video,
                is_webrtc
            ));
            for invalid in [
                "audio/webm; codecs=opus",
                "text/plain",
                "video/webm; codecs=\"vp8,vp9\"",
                "video/webm; codecs=\"vp8 vp9\"",
                "video/webm; codecs=\"vp8é\"",
                "video/webm; codecs=\"\u{2003}vp8\"",
                "video/VP8\"",
                "video/vp8;",
            ] {
                assert!(
                    !is_valid_media_capabilities_mime_type(invalid, MediaKind::Video, is_webrtc),
                    "{invalid}"
                );
            }
        }
    }
}
