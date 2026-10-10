//! Conversion and validation for the local RTP frontend. The capability tables
//! describe compatibility; they do not advertise a running media codec backend.

use std::collections::BTreeMap;

use crate::webidl;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, webidl::WebIdlDictionary)]
#[serde(rename_all = "camelCase")]
#[webidl(prefix = "RTCRtpCodec")]
pub(super) struct Codec {
    #[webidl(required)]
    pub mime_type: String,
    #[webidl(required)]
    pub clock_rate: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdp_fmtp_line: Option<String>,
}

impl Codec {
    pub(super) fn matches(&self, other: &Self) -> bool {
        self.mime_type.eq_ignore_ascii_case(&other.mime_type)
            && self.clock_rate == other.clock_rate
            && self.channels == other.channels
            && match (&self.sdp_fmtp_line, &other.sdp_fmtp_line) {
                (None, None) => true,
                (Some(left), Some(right)) => {
                    match (fmtp_parameters(left), fmtp_parameters(right)) {
                        (Some(left), Some(right)) => left == right,
                        _ => left == right,
                    }
                }
                _ => false,
            }
    }

    pub(super) fn is_media(&self) -> bool {
        !self.mime_type.split('/').nth(1).is_some_and(|name| {
            ["rtx", "red", "ulpfec", "flexfec-03", "cn"]
                .iter()
                .any(|auxiliary| name.eq_ignore_ascii_case(auxiliary))
        })
    }
}

// Compare key/value FMTP independent of parameter order and surrounding ASCII
// whitespace. Opaque codec-specific lists (e.g. telephone-event) stay exact.
// Missing parameters are not invented and asymmetric values are not discarded.
fn fmtp_parameters(value: &str) -> Option<BTreeMap<&str, &str>> {
    let mut parameters = BTreeMap::new();
    for part in value.split(';') {
        let (name, value) = part.trim().split_once('=')?;
        let name = name.trim();
        if name.is_empty() || parameters.insert(name, value.trim()).is_some() {
            return None;
        }
    }
    Some(parameters)
}

pub(super) fn capabilities(kind: &str) -> Vec<Codec> {
    #[derive(serde::Deserialize)]
    struct Capabilities {
        codecs: Vec<Codec>,
    }
    serde_json::from_str::<Capabilities>(super::rtp_capabilities::capabilities_json(kind))
        .expect("static RTP capability table")
        .codecs
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCRtpCodingParameters")]
pub(super) struct Coding {
    pub rid: Option<String>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCRtpEncodingParameters")]
pub(super) struct Encoding {
    #[webidl(inherit)]
    pub coding: Coding,
    #[webidl(default = true)]
    pub active: bool,
    #[webidl(dictionary)]
    pub codec: Option<Codec>,
    pub max_bitrate: Option<u32>,
    #[webidl(converter = "raw")]
    pub max_framerate: Option<webidl::Double>,
    #[webidl(converter = "raw")]
    pub scale_resolution_down_by: Option<webidl::Double>,
}

impl Encoding {
    pub(super) fn snapshot(&self) -> serde_json::Value {
        let mut value = serde_json::json!({"active": self.active});
        for (name, member) in [
            (
                "rid",
                self.coding.rid.as_ref().map(|v| serde_json::json!(v)),
            ),
            ("codec", self.codec.as_ref().map(|v| serde_json::json!(v))),
            ("maxBitrate", self.max_bitrate.map(|v| serde_json::json!(v))),
            (
                "maxFramerate",
                self.max_framerate.as_ref().map(|v| serde_json::json!(v.0)),
            ),
            (
                "scaleResolutionDownBy",
                self.scale_resolution_down_by
                    .as_ref()
                    .map(|v| serde_json::json!(v.0)),
            ),
        ] {
            if let Some(member) = member {
                value[name] = member;
            }
        }
        value
    }
}

#[derive(serde::Serialize, webidl::WebIdlDictionary)]
#[serde(rename_all = "camelCase")]
#[webidl(prefix = "RTCRtcpParameters")]
pub(super) struct Rtcp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduced_size: Option<bool>,
}

#[derive(serde::Serialize, webidl::WebIdlDictionary)]
#[serde(rename_all = "camelCase")]
#[webidl(prefix = "RTCRtpHeaderExtensionParameters")]
pub(super) struct HeaderExtension {
    #[webidl(default = false)]
    encrypted: bool,
    #[webidl(required)]
    id: u16,
    #[webidl(required)]
    uri: String,
}

#[derive(serde::Serialize, webidl::WebIdlDictionary)]
#[serde(rename_all = "camelCase")]
#[webidl(prefix = "RTCRtpCodecParameters")]
pub(super) struct CodecParameters {
    #[webidl(inherit)]
    #[serde(flatten)]
    codec: Codec,
    #[webidl(required, converter = "octet")]
    payload_type: u8,
}

#[derive(serde::Serialize, webidl::WebIdlDictionary)]
#[serde(rename_all = "camelCase")]
#[webidl(prefix = "RTCRtpParameters")]
pub(super) struct Parameters {
    #[webidl(required, sequence, converter = "dictionary")]
    codecs: Vec<CodecParameters>,
    #[webidl(required, sequence, converter = "dictionary")]
    header_extensions: Vec<HeaderExtension>,
    #[webidl(required, dictionary)]
    rtcp: Rtcp,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCRtpSendParameters")]
pub(super) struct SendParameters {
    #[webidl(inherit)]
    pub parameters: Parameters,
    #[webidl(required, sequence, converter = "dictionary")]
    pub encodings: Vec<Encoding>,
    #[webidl(required)]
    pub transaction_id: String,
}

impl SendParameters {
    pub(super) fn snapshot(&self) -> serde_json::Value {
        let mut result = serde_json::to_value(&self.parameters).expect("RTP parameters");
        result["transactionId"] = serde_json::json!(self.transaction_id);
        result["encodings"] = self.encodings.iter().map(Encoding::snapshot).collect();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_matching_preserves_parameters_but_ignores_fmtp_order() {
        let mut left = capabilities("audio")[0].clone();
        let mut right = left.clone();
        right.mime_type = "AUDIO/OPUS".into();
        right.sdp_fmtp_line = Some("useinbandfec=1 ; minptime = 10".into());
        assert!(left.matches(&right));
        right.sdp_fmtp_line = Some("useinbandfec=1;minptime=20".into());
        assert!(!left.matches(&right));
        left.sdp_fmtp_line = None;
        assert!(!left.matches(&right));
        assert_eq!(fmtp_parameters("a=1;a=2"), None);
        assert_eq!(fmtp_parameters("0-15"), None);
    }
}
