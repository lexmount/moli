//! Transport-independent request and response types shared by the parser,
//! Fetch, Service Workers and Cache Storage.
//!
//! These values contain no execution context or transport handles. In
//! particular, delivery through a Service Worker is a property of a fetch,
//! not a replacement for the response's established filtering or TAO result.

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    strum::AsRefStr,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum FetchPriorityHint {
    Low,
    #[default]
    Auto,
    High,
}

impl FetchPriorityHint {
    pub fn from_attribute(value: Option<&str>) -> Option<Self> {
        // HTML `fetchpriority` attributes are ASCII case-insensitive, while
        // RequestInit.priority uses the case-sensitive WebIDL enum parser.
        let value = value?.trim();
        if value.eq_ignore_ascii_case("low") {
            Some(Self::Low)
        } else if value.eq_ignore_ascii_case("auto") {
            Some(Self::Auto)
        } else if value.eq_ignore_ascii_case("high") {
            Some(Self::High)
        } else {
            None
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::AsRefStr,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum RequestCredentialsMode {
    Include,
    SameOrigin,
    Omit,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::AsRefStr,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum RequestMode {
    Navigate,
    SameOrigin,
    NoCors,
    Cors,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::AsRefStr, strum::EnumString, strum::IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum RequestRedirectMode {
    Follow,
    Error,
    Manual,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FetchResponseType {
    #[default]
    Default,
    Basic,
    Cors,
    Error,
    Opaque,
    OpaqueRedirect,
}

impl FetchResponseType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Basic => "basic",
            Self::Cors => "cors",
            Self::Error => "error",
            Self::Opaque => "opaque",
            Self::OpaqueRedirect => "opaqueredirect",
        }
    }

    pub fn is_readable(self) -> bool {
        matches!(self, Self::Default | Self::Basic | Self::Cors)
    }

    /// Defaults for synthetic responses and old persisted records that did
    /// not retain the Fetch algorithm's timing authorization.
    pub fn default_timing_allow_passed(self) -> bool {
        matches!(self, Self::Default | Self::Basic)
    }
}

impl std::str::FromStr for FetchResponseType {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "default" => Ok(Self::Default),
            "basic" => Ok(Self::Basic),
            "cors" => Ok(Self::Cors),
            "error" => Ok(Self::Error),
            "opaque" => Ok(Self::Opaque),
            "opaqueredirect" => Ok(Self::OpaqueRedirect),
            _ => Err("invalid Fetch response type"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchResponseMetadata {
    /// Frozen CORS exposed-name list, retained through filtering, cloning and cache storage.
    #[serde(default)]
    pub cors_exposed_header_names: Option<Vec<String>>,
    pub response_type: FetchResponseType,
    /// Internal URLs, including those hidden by an opaque response's public URL.
    pub url_list: Vec<Url>,
    pub timing_allow_passed: bool,
    /// Retained independently for old cache records containing only the final
    /// URL. Their redirect history cannot be reconstructed from that URL.
    pub redirected: bool,
}

impl Default for FetchResponseMetadata {
    fn default() -> Self {
        Self {
            cors_exposed_header_names: None,
            response_type: FetchResponseType::Default,
            url_list: Vec::new(),
            timing_allow_passed: true,
            redirected: false,
        }
    }
}

impl FetchResponseMetadata {
    pub fn final_url(&self) -> Option<&Url> {
        self.url_list.last()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FetchResponseHead {
    pub metadata: FetchResponseMetadata,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, Vec<u8>)>,
}

impl FetchResponseHead {
    pub fn with_body(self, body: Vec<u8>) -> FetchResponse {
        FetchResponse { head: self, body }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FetchResponse {
    pub head: FetchResponseHead,
    pub body: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::FetchPriorityHint;

    #[test]
    fn fetch_priority_hint_parsing_keeps_webidl_strict_and_html_case_insensitive() {
        assert_eq!(
            "high".parse::<FetchPriorityHint>().ok(),
            Some(FetchPriorityHint::High)
        );
        assert!(
            "HIGH".parse::<FetchPriorityHint>().is_err(),
            "RequestInit priority is a WebIDL enum and must stay case-sensitive"
        );
        assert_eq!(
            FetchPriorityHint::from_attribute(Some(" HIGH ")),
            Some(FetchPriorityHint::High),
            "HTML fetchpriority attribute keywords are ASCII case-insensitive"
        );
    }
}

pub mod header_policy;

impl FetchResponseMetadata {
    pub fn capture_cors_exposed_headers(&mut self, headers: &[(String, Vec<u8>)]) {
        if self.response_type == FetchResponseType::Cors && self.cors_exposed_header_names.is_none()
        {
            let names = headers
                .iter()
                .filter(|(name, _)| self.exposes_header(headers, name))
                .map(|(name, _)| name.to_ascii_lowercase())
                .collect::<std::collections::BTreeSet<_>>();
            self.cors_exposed_header_names = Some(names.into_iter().collect());
        }
    }
}

impl FetchResponseMetadata {
    // This earlier phase has no captured request credentials fact. A raw wildcard
    // cannot prove credentialless exposure; explicit frozen names remain authoritative.
    pub fn exposes_header(&self, headers: &[(String, Vec<u8>)], name: &str) -> bool {
        if !self.response_type.is_readable() {
            return false;
        }
        if self.response_type == FetchResponseType::Default {
            return true;
        }
        if name.eq_ignore_ascii_case("set-cookie") || name.eq_ignore_ascii_case("set-cookie2") {
            return false;
        }
        if self.response_type == FetchResponseType::Basic {
            return true;
        }
        if [
            "cache-control",
            "content-language",
            "content-length",
            "content-type",
            "expires",
            "last-modified",
            "pragma",
        ]
        .iter()
        .any(|safe| name.eq_ignore_ascii_case(safe))
        {
            return true;
        }
        if let Some(names) = &self.cors_exposed_header_names {
            return names
                .iter()
                .any(|exposed| exposed.eq_ignore_ascii_case(name));
        }
        headers
            .iter()
            .filter(|(field, _)| field.eq_ignore_ascii_case("access-control-expose-headers"))
            .flat_map(|(_, value)| value.split(|byte| *byte == b','))
            .map(|token| token.trim_ascii())
            .any(|token| token.eq_ignore_ascii_case(name.as_bytes()))
    }
}
