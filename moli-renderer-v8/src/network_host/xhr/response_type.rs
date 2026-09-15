use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub(super) enum XmlHttpRequestResponseType {
    #[strum(serialize = "")]
    Default,
    ArrayBuffer,
    Blob,
    Document,
    Json,
    Text,
}

impl XmlHttpRequestResponseType {
    pub(super) fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub(super) fn label(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
mod tests {
    use super::XmlHttpRequestResponseType;

    #[test]
    fn xhr_response_type_parses_standard_tokens() {
        for (raw, expected) in [
            ("", XmlHttpRequestResponseType::Default),
            ("arraybuffer", XmlHttpRequestResponseType::ArrayBuffer),
            ("blob", XmlHttpRequestResponseType::Blob),
            ("document", XmlHttpRequestResponseType::Document),
            ("json", XmlHttpRequestResponseType::Json),
            ("text", XmlHttpRequestResponseType::Text),
        ] {
            let parsed = XmlHttpRequestResponseType::parse(raw)
                .expect("standard XHR responseType token should parse");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.label(), raw);
        }
    }

    #[test]
    fn xhr_response_type_rejects_non_standard_tokens() {
        assert!(XmlHttpRequestResponseType::parse("JSON").is_none());
        assert!(XmlHttpRequestResponseType::parse("buffer").is_none());
        assert!(XmlHttpRequestResponseType::parse(" text ").is_none());
    }

    #[test]
    fn xhr_response_type_rejects_historical_moz_tokens() {
        for raw in ["moz-blob", "moz-chunked-text", "moz-chunked-arraybuffer"] {
            assert!(XmlHttpRequestResponseType::parse(raw).is_none());
        }
    }
}
