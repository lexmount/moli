//! SDP syntax and structured session parsing. JSEP negotiation and semantic
//! validation belong to the description application step, not this parser.

use webrtc_sdp::{
    SdpLine, SdpSession, SdpType, error::SdpParserError, media_type::parse_media_vector,
    parse_sdp_line,
};

#[derive(Debug)]
pub(super) struct SyntaxError {
    pub message: String,
    pub line_number: i32,
}

impl SyntaxError {
    fn at(message: impl Into<String>, index: usize) -> Self {
        Self {
            message: message.into(),
            line_number: i32::try_from(index.saturating_add(1)).unwrap_or(i32::MAX),
        }
    }
}

impl From<SdpParserError> for SyntaxError {
    fn from(error: SdpParserError) -> Self {
        let index = match &error {
            SdpParserError::Line { line_number, .. }
            | SdpParserError::Unsupported { line_number, .. }
            | SdpParserError::Sequence { line_number, .. } => *line_number,
        };
        Self::at(error.to_string(), index)
    }
}

pub(super) fn parse(text: &str) -> Result<SdpSession, SyntaxError> {
    let mut lines = Vec::new();
    let mut warnings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_sdp_line(line, index) {
            Ok(line) => lines.push(line),
            // RFC 8866 requires receivers to ignore unrecognized attributes.
            // Keep them as diagnostics; the original SDP is never serialized
            // back from this representation or stripped of these attributes.
            Err(error @ SdpParserError::Unsupported { .. }) if line.starts_with("a=") => {
                warnings.push(error);
            }
            Err(error) => return Err(error.into()),
        }
    }
    let eof = text.lines().count();
    let mut lines = lines.into_iter();
    let header = |lines: &mut std::vec::IntoIter<SdpLine>| {
        lines
            .next()
            .ok_or_else(|| SyntaxError::at("Missing required SDP session header.", eof))
    };
    let version = header(&mut lines)?;
    let origin = header(&mut lines)?;
    let session = header(&mut lines)?;
    let SdpType::Version(version) = version.sdp_type else {
        return Err(SyntaxError::at(
            "Expected SDP version.",
            version.line_number,
        ));
    };
    let SdpType::Origin(origin) = origin.sdp_type else {
        return Err(SyntaxError::at("Expected SDP origin.", origin.line_number));
    };
    let SdpType::Session(session) = session.sdp_type else {
        return Err(SyntaxError::at(
            "Expected SDP session name.",
            session.line_number,
        ));
    };
    let mut parsed = SdpSession::new(version, origin, session);
    let mut lines: Vec<_> = lines.collect();
    let media_start = lines
        .iter()
        .position(|line| matches!(line.sdp_type, SdpType::Media(_)))
        .unwrap_or(lines.len());
    let mut media = lines.split_off(media_start);
    parsed.parse_session_vector(&mut lines)?;
    if parsed.timing.is_none() {
        let index = media.first().map_or(eof, |line| line.line_number);
        return Err(SyntaxError::at("Missing SDP timing.", index));
    }
    if !media.is_empty() {
        parsed.extend_media(parse_media_vector(&mut media)?);
    }
    parsed.warnings = warnings;
    // Use the parser's public session/media APIs rather than parse_sdp(): its
    // 51-byte minimum rejects valid short sessions, and its final sanity check
    // mixes syntax with semantic constraints (e.g. RID payload references).
    Ok(parsed)
}

pub(super) fn session_header(mids: &[&str]) -> String {
    let mut text = "v=0\r\no=- 0 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n".to_owned();
    // BUNDLE requires at least one identification-tag. A connection without
    // media or a data channel has no group to advertise.
    if !mids.is_empty() {
        text.push_str(&format!("a=group:BUNDLE {}\r\n", mids.join(" ")));
    }
    text.push_str("a=extmap-allow-mixed\r\na=msid-semantic: WMS\r\n");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use webrtc_sdp::attribute_type::{SdpAttribute, SdpAttributeType};

    const SHORT: &str = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n";

    #[test]
    fn sdp_short_session_is_valid_without_padding_or_media() {
        assert!(SHORT.len() < 51);
        let parsed = parse(SHORT).unwrap();
        assert_eq!(parsed.version, 0);
        assert!(parsed.timing.is_some());
        assert!(parsed.media.is_empty());
    }

    #[test]
    fn sdp_parser_retains_unknown_attributes_as_warnings() {
        let text = format!("{SHORT}a=x-vendor-extension:unchanged\r\n");
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed.warnings.len(), 1);
        assert!(matches!(
            parsed.warnings[0],
            SdpParserError::Unsupported { line_number: 4, .. }
        ));
    }

    #[test]
    fn sdp_syntax_errors_report_original_one_based_lines() {
        for (text, line) in [
            ("", 1),
            ("v=0\r\n", 2),
            ("s=-\r\no=- 0 0 IN IP4 0.0.0.0\r\nv=0\r\nt=0 0\r\n", 1),
            ("v=0\r\no=- invalid 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n", 2),
            ("v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\n", 4),
        ] {
            assert_eq!(parse(text).unwrap_err().line_number, line, "{text:?}");
        }
        let text = format!("{SHORT}\r\nm=audio invalid UDP/TLS/RTP/SAVPF 111\r\n");
        assert_eq!(parse(&text).unwrap_err().line_number, 6);
    }

    #[test]
    fn sdp_structured_media_is_separate_from_jsep_semantics() {
        let text = format!(
            "{SHORT}m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=mid:voice\r\na=sendonly\r\na=rtpmap:111 opus/48000/2\r\na=rid:high send pt=112\r\n"
        );
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed.media.len(), 1);
        assert!(
            matches!(parsed.media[0].get_attribute(SdpAttributeType::Mid), Some(SdpAttribute::Mid(mid)) if mid == "voice")
        );
        // A RID referring to an absent payload is a semantic failure, not a
        // reason to classify an otherwise parsed description as syntax-error.
        assert!(
            parsed.media[0]
                .get_attribute(SdpAttributeType::Rid)
                .is_some()
        );
    }

    #[test]
    fn sdp_all_compatibility_offers_parse_and_empty_bundle_is_omitted() {
        for bits in 0..8 {
            let text = super::super::build_signaling_only_offer(
                bits & 1 != 0,
                bits & 2 != 0,
                bits & 4 != 0,
            );
            let parsed = parse(&text).unwrap();
            assert_eq!(parsed.media.len(), (bits as u32).count_ones() as usize);
            assert_eq!(text.contains("a=group:BUNDLE"), bits != 0);
        }
        let text = super::super::rtp_offer::build(&[], false);
        assert!(parse(&text).unwrap().media.is_empty());
        assert!(!text.contains("a=group:BUNDLE"));
    }
}
