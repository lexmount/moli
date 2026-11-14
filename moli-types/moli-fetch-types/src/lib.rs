//! Response state shared by Fetch, Service Workers and Cache Storage.
//!
//! These values contain no execution context or transport handles. In
//! particular, delivery through a Service Worker is a property of a fetch,
//! not a replacement for the response's established filtering or TAO result.

use serde::{Deserialize, Serialize};
use url::Url;

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
