//! Adapt the existing signaling-only SDP to local transceiver preferences.
//! Payload identities and RTX dependencies come from the compatibility offer;
//! this does not allocate ICE/DTLS transports or negotiate a remote description.

use super::rtp_parameters::Codec;

pub(super) struct Section {
    pub kind: String,
    pub direction: String,
    pub codecs: Vec<Codec>,
    pub streams: Vec<String>,
    pub track: String,
}

struct Format<'a> {
    payload: &'a str,
    codec: Codec,
    lines: Vec<&'a str>,
    apt: Option<&'a str>,
}

fn formats<'a>(section: &'a str, kind: &str) -> Vec<Format<'a>> {
    section
        .lines()
        .filter_map(|line| {
            let (payload, encoding) = line.strip_prefix("a=rtpmap:")?.split_once(' ')?;
            let mut fields = encoding.split('/');
            let name = fields.next()?;
            let clock_rate = fields.next()?.parse().ok()?;
            let channels = if kind == "audio" {
                Some(fields.next().unwrap_or("1").parse().ok()?)
            } else {
                None
            };
            let fmtp_prefix = format!("a=fmtp:{payload} ");
            let fmtp = section
                .lines()
                .find_map(|line| line.strip_prefix(&fmtp_prefix));
            let apt = fmtp.and_then(|value| value.strip_prefix("apt="));
            let lines = section
                .lines()
                .filter(|line| {
                    line.strip_prefix("a=rtpmap:")
                        .or_else(|| line.strip_prefix("a=fmtp:"))
                        .or_else(|| line.strip_prefix("a=rtcp-fb:"))
                        .is_some_and(|value| {
                            value.split_once(' ').is_some_and(|(pt, _)| pt == payload)
                        })
                })
                .collect();
            Some(Format {
                payload,
                codec: Codec {
                    mime_type: format!("{kind}/{name}"),
                    clock_rate,
                    channels,
                    sdp_fmtp_line: if apt.is_some() || name.eq_ignore_ascii_case("red") {
                        None
                    } else {
                        fmtp.map(str::to_owned)
                    },
                },
                lines,
                apt,
            })
        })
        .collect()
}

fn media_section(kind: &str, direction: &str, codecs: &[Codec], mid: usize) -> String {
    let template = if kind == "audio" {
        super::RTC_AUDIO_OFFER_SECTION
    } else {
        super::RTC_VIDEO_OFFER_SECTION
    };
    let formats = formats(template, kind);
    let mut selected: Vec<&Format<'_>> = Vec::new();
    for codec in codecs {
        for format in &formats {
            if codec.matches(&format.codec)
                && !selected
                    .iter()
                    .any(|selected| selected.payload == format.payload)
            {
                selected.push(format);
            }
        }
    }
    // RTX is only meaningful when its referenced primary payload is offered.
    let primary: Vec<_> = selected
        .iter()
        .filter(|format| format.apt.is_none())
        .map(|format| format.payload)
        .collect();
    selected.retain(|format| format.apt.is_none_or(|apt| primary.contains(&apt)));
    let payloads: Vec<_> = selected.iter().map(|format| format.payload).collect();
    let mut result = format!(
        "m={kind} 9 UDP/TLS/RTP/SAVPF {}\r\nc=IN IP4 0.0.0.0\r\na=mid:{mid}\r\na={direction}\r\na=rtcp-mux\r\na=rtcp-rsize\r\n",
        payloads.join(" ")
    );
    for format in selected {
        for line in &format.lines {
            result.push_str(line);
            result.push_str("\r\n");
        }
    }
    result
}

pub(super) fn build(sections: &[Section], data: bool) -> String {
    let count = sections.len() + usize::from(data);
    let mids: Vec<_> = (0..count).map(|mid| mid.to_string()).collect();
    let mut sdp = format!(
        "v=0\r\no=- 0 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE {}\r\na=extmap-allow-mixed\r\na=msid-semantic: WMS\r\n",
        mids.join(" ")
    );
    for (mid, section) in sections.iter().enumerate() {
        sdp.push_str(&media_section(
            &section.kind,
            &section.direction,
            &section.codecs,
            mid,
        ));
        if matches!(section.direction.as_str(), "sendrecv" | "sendonly") {
            if section.streams.is_empty() {
                sdp.push_str(&format!("a=msid:- {}\r\n", section.track));
            } else {
                for stream in &section.streams {
                    sdp.push_str(&format!("a=msid:{stream} {}\r\n", section.track));
                }
            }
        }
    }
    if data {
        sdp.push_str(
            &super::RTC_DATA_OFFER_SECTION.replace("a=mid:2", &format!("a=mid:{}", sections.len())),
        );
    }
    sdp
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codec_order_filters_payloads_and_rtx_and_uses_unique_mids() {
        let all = super::super::rtp_parameters::capabilities("video");
        let h264 = all
            .iter()
            .find(|codec| codec.mime_type == "video/H264")
            .unwrap()
            .clone();
        let rtx = all
            .iter()
            .find(|codec| codec.mime_type == "video/rtx")
            .unwrap()
            .clone();
        let sdp = build(
            &[
                Section {
                    kind: "video".into(),
                    direction: "sendonly".into(),
                    codecs: vec![h264.clone(), rtx],
                    streams: vec!["s1".into(), "s2".into()],
                    track: "track".into(),
                },
                Section {
                    kind: "video".into(),
                    direction: "inactive".into(),
                    codecs: vec![h264],
                    streams: vec![],
                    track: "inactive".into(),
                },
            ],
            true,
        );
        assert!(sdp.contains("a=group:BUNDLE 0 1 2\r\n"));
        assert!(sdp.contains("m=video 9 UDP/TLS/RTP/SAVPF 103 104\r\n"));
        assert!(!sdp.contains("VP8"));
        assert!(sdp.contains("a=fmtp:104 apt=103\r\n"));
        assert_eq!(sdp.matches("a=mid:").count(), 3);
        assert!(sdp.contains("a=msid:s1 track\r\na=msid:s2 track\r\n"));
        assert!(!sdp.contains("a=msid:- inactive"));
    }
}
