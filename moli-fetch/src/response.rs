use anyhow::{Result, anyhow};
use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
use url::Url;

use crate::{StreamingHtmlResponse, StreamingRawResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegotiatedHttpVersion {
    Http09,
    Http10,
    Http11,
    Http2,
    Http3,
}

impl NegotiatedHttpVersion {
    pub fn protocol_name(self) -> &'static str {
        match self {
            Self::Http09 => "http/0.9",
            Self::Http10 => "http/1.0",
            Self::Http11 => "http/1.1",
            Self::Http2 => "h2",
            Self::Http3 => "h3",
        }
    }

    pub(crate) fn from_status_line(line: &str) -> Option<Self> {
        let version = line.strip_prefix("HTTP/")?.split_whitespace().next()?;
        match version {
            "0.9" => Some(Self::Http09),
            "1.0" => Some(Self::Http10),
            "1.1" => Some(Self::Http11),
            "2" | "2.0" => Some(Self::Http2),
            "3" | "3.0" => Some(Self::Http3),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResponseHead {
    pub final_url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<RedirectInfo>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
}

#[derive(Debug)]
pub enum ResponseBody {
    MaterializedText {
        text: String,
        exact_bytes: Option<Vec<u8>>,
    },
    MaterializedBytes(Vec<u8>),
    StreamingText(Box<StreamingHtmlResponse>),
    StreamingBytes(Box<StreamingRawResponse>),
}

impl ResponseBody {
    pub fn materialized_text(text: String, bytes: Vec<u8>) -> Self {
        let exact_bytes = (!bytes.is_empty()).then_some(bytes);
        Self::MaterializedText { text, exact_bytes }
    }

    /// Builds a text body from exact bytes without retaining a second copy
    /// when the bytes are already valid UTF-8.
    pub fn lossy_text_from_bytes(bytes: Vec<u8>) -> Self {
        match String::from_utf8(bytes) {
            Ok(text) => Self::MaterializedText {
                text,
                exact_bytes: None,
            },
            Err(error) => {
                let bytes = error.into_bytes();
                let text = String::from_utf8_lossy(&bytes).into_owned();
                Self::MaterializedText {
                    text,
                    exact_bytes: Some(bytes),
                }
            }
        }
    }

    pub fn materialized_bytes(bytes: Vec<u8>) -> Self {
        Self::MaterializedBytes(bytes)
    }

    pub fn is_streaming(&self) -> bool {
        matches!(self, Self::StreamingText(_) | Self::StreamingBytes(_))
    }

    pub fn try_into_materialized_bytes(self) -> std::result::Result<Vec<u8>, Self> {
        match self {
            Self::MaterializedBytes(bytes) => Ok(bytes),
            Self::MaterializedText { text, exact_bytes } => {
                Ok(exact_bytes.unwrap_or_else(|| text.into_bytes()))
            }
            Self::StreamingText(_) | Self::StreamingBytes(_) => Err(self),
        }
    }

    pub fn as_materialized_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::MaterializedBytes(bytes) => Some(bytes),
            Self::MaterializedText { text, exact_bytes } => {
                Some(exact_bytes.as_deref().unwrap_or(text.as_bytes()))
            }
            Self::StreamingText(_) | Self::StreamingBytes(_) => None,
        }
    }

    pub fn clone_materialized(&self) -> Option<Self> {
        match self {
            Self::MaterializedBytes(bytes) => Some(Self::MaterializedBytes(bytes.clone())),
            Self::MaterializedText { text, exact_bytes } => Some(Self::MaterializedText {
                text: text.clone(),
                exact_bytes: exact_bytes.clone(),
            }),
            Self::StreamingText(_) | Self::StreamingBytes(_) => None,
        }
    }

    pub fn as_materialized_text(&self) -> Option<&str> {
        match self {
            Self::MaterializedText { text, .. } => Some(text),
            Self::MaterializedBytes(_) | Self::StreamingText(_) | Self::StreamingBytes(_) => None,
        }
    }

    pub fn try_into_lossy_materialized_text(self) -> std::result::Result<(String, Vec<u8>), Self> {
        match self {
            Self::MaterializedText { text, exact_bytes } => {
                let bytes = exact_bytes.unwrap_or_else(|| text.as_bytes().to_vec());
                Ok((text, bytes))
            }
            Self::MaterializedBytes(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                Ok((text, bytes))
            }
            Self::StreamingText(_) | Self::StreamingBytes(_) => Err(self),
        }
    }

    /// Converts an already-materialized body to its text representation.
    pub fn try_into_materialized_text_body(self) -> std::result::Result<Self, Self> {
        match self {
            Self::MaterializedText { .. } => Ok(self),
            Self::MaterializedBytes(bytes) => Ok(Self::lossy_text_from_bytes(bytes)),
            Self::StreamingText(_) | Self::StreamingBytes(_) => Err(self),
        }
    }

    /// Drains any streaming source and returns a complete byte body.
    ///
    /// This is an explicit compatibility boundary. Prefer chunked consumption
    /// when the caller can avoid building a full response body in memory.
    pub async fn into_materialized_bytes(self) -> Result<Vec<u8>> {
        match self {
            Self::MaterializedBytes(bytes) => Ok(bytes),
            Self::MaterializedText { text, exact_bytes } => {
                Ok(exact_bytes.unwrap_or_else(|| text.into_bytes()))
            }
            Self::StreamingText(response) => {
                let mut response = *response;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.next_chunk().await {
                    bytes.extend_from_slice(chunk.as_bytes());
                }
                response.finish().await?;
                Ok(bytes)
            }
            Self::StreamingBytes(response) => {
                let mut response = *response;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.next_chunk().await {
                    bytes.extend_from_slice(&chunk);
                }
                response.finish().await?;
                Ok(bytes)
            }
        }
    }

    /// Drains any streaming source and returns a complete lossy text body plus
    /// the exact bytes used to derive it.
    pub async fn into_lossy_materialized_text(self) -> Result<(String, Vec<u8>)> {
        match self {
            Self::MaterializedText { text, exact_bytes } => {
                let bytes = exact_bytes.unwrap_or_else(|| text.as_bytes().to_vec());
                Ok((text, bytes))
            }
            Self::MaterializedBytes(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                Ok((text, bytes))
            }
            Self::StreamingText(response) => {
                let mut response = *response;
                let mut text = String::new();
                while let Some(chunk) = response.next_chunk().await {
                    text.push_str(&chunk);
                }
                response.finish().await?;
                let bytes = text.as_bytes().to_vec();
                Ok((text, bytes))
            }
            Self::StreamingBytes(response) => {
                let bytes = Self::StreamingBytes(response)
                    .into_materialized_bytes()
                    .await?;
                let text = String::from_utf8_lossy(&bytes).into_owned();
                Ok((text, bytes))
            }
        }
    }

    /// Drains a body source and returns a materialized text body.
    pub async fn into_materialized_text_body(self) -> Result<Self> {
        match self {
            Self::MaterializedText { .. } => Ok(self),
            Self::MaterializedBytes(bytes) => Ok(Self::lossy_text_from_bytes(bytes)),
            Self::StreamingText(response) => {
                let mut response = *response;
                let mut text = String::new();
                while let Some(chunk) = response.next_chunk().await {
                    text.push_str(&chunk);
                }
                response.finish().await?;
                Ok(Self::MaterializedText {
                    text,
                    exact_bytes: None,
                })
            }
            Self::StreamingBytes(response) => {
                let bytes = Self::StreamingBytes(response)
                    .into_materialized_bytes()
                    .await?;
                Ok(Self::lossy_text_from_bytes(bytes))
            }
        }
    }
}

#[derive(Debug)]
pub struct Response {
    pub final_url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    body: ResponseBody,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<RedirectInfo>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_extra_info: Option<NetworkRequestExtraInfo>,
}

impl Clone for Response {
    fn clone(&self) -> Self {
        Self {
            final_url: self.final_url.clone(),
            status: self.status,
            headers: self.headers.clone(),
            body: self
                .body
                .clone_materialized()
                .expect("Response body should remain materialized"),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.clone(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
            network_request_extra_info: self.network_request_extra_info.clone(),
        }
    }
}

impl Response {
    pub fn with_network_request_extra_info(
        mut self,
        network_request_extra_info: Option<NetworkRequestExtraInfo>,
    ) -> Self {
        self.network_request_extra_info = network_request_extra_info;
        self
    }

    pub fn network_request_extra_info(&self) -> Option<&NetworkRequestExtraInfo> {
        self.network_request_extra_info.as_ref()
    }

    pub fn body_text(&self) -> &str {
        self.body
            .as_materialized_text()
            .expect("Response body should remain materialized text")
    }

    pub fn body_bytes(&self) -> &[u8] {
        self.body
            .as_materialized_bytes()
            .expect("Response body should remain materialized")
    }

    /// Clones the complete materialized byte payload for compatibility callers
    /// whose public contract requires an owned full-body buffer.
    pub fn clone_body_bytes(&self) -> Vec<u8> {
        self.body_bytes().to_vec()
    }

    pub fn materialized_body(&self) -> ResponseBody {
        self.body
            .clone_materialized()
            .expect("Response body should remain materialized")
    }

    pub fn head(&self) -> ResponseHead {
        ResponseHead {
            final_url: self.final_url.clone(),
            status: self.status,
            headers: self.headers.clone(),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.clone(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        }
    }

    pub fn from_head_and_body(head: ResponseHead, body: String, body_bytes: Vec<u8>) -> Self {
        Self::from_head_and_materialized_body(
            head,
            ResponseBody::materialized_text(body, body_bytes),
        )
        .expect("materialized text body should build a Response")
    }

    pub fn from_head_and_text_body(head: ResponseHead, body: String) -> Self {
        Self::from_head_and_body(head, body, Vec::new())
    }

    /// Builds a materialized text response from exact bytes by deriving the
    /// compatibility text view with UTF-8 replacement semantics.
    pub fn from_head_and_lossy_body_bytes(head: ResponseHead, body_bytes: Vec<u8>) -> Self {
        Self::from_head_and_materialized_body(head, ResponseBody::lossy_text_from_bytes(body_bytes))
            .expect("materialized text body should build a Response")
    }

    pub fn from_head_and_materialized_body(head: ResponseHead, body: ResponseBody) -> Result<Self> {
        let body = body.try_into_materialized_text_body().map_err(|_| {
            anyhow!("cannot build materialized Response from streaming response body")
        })?;
        Ok(Self {
            final_url: head.final_url,
            status: head.status,
            headers: head.headers,
            body,
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            redirected: head.redirected,
            redirect_chain: head.redirect_chain,
            from_cache: head.from_cache,
            negotiated_http_version: head.negotiated_http_version,
            network_request_extra_info: None,
        })
    }

    pub async fn from_head_and_body_source(head: ResponseHead, body: ResponseBody) -> Result<Self> {
        let body = body.into_materialized_text_body().await?;
        Ok(Self {
            final_url: head.final_url,
            status: head.status,
            headers: head.headers,
            body,
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            redirected: head.redirected,
            redirect_chain: head.redirect_chain,
            from_cache: head.from_cache,
            negotiated_http_version: head.negotiated_http_version,
            network_request_extra_info: None,
        })
    }

    pub fn into_parts(self) -> (ResponseHead, String, Vec<u8>) {
        let head = ResponseHead {
            final_url: self.final_url,
            status: self.status,
            headers: self.headers,
            request_cookie_report: self.request_cookie_report,
            cookie_set_reports: self.cookie_set_reports,
            redirected: self.redirected,
            redirect_chain: self.redirect_chain,
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        };
        let (body, body_bytes) = self
            .body
            .try_into_lossy_materialized_text()
            .expect("Response body should remain materialized text");
        (head, body, body_bytes)
    }

    /// Consumes a materialized response when the next layer only accepts text
    /// and intentionally discards the exact byte payload.
    pub fn into_text_parts(self) -> (ResponseHead, String) {
        let (head, body, _) = self.into_parts();
        (head, body)
    }

    pub fn into_body(self) -> (ResponseHead, ResponseBody) {
        let head = ResponseHead {
            final_url: self.final_url,
            status: self.status,
            headers: self.headers,
            request_cookie_report: self.request_cookie_report,
            cookie_set_reports: self.cookie_set_reports,
            redirected: self.redirected,
            redirect_chain: self.redirect_chain,
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        };
        (head, self.body)
    }

    pub fn into_materialized_raw_response(self) -> RawResponse {
        let network_request_extra_info = self.network_request_extra_info.clone();
        let (head, body) = self.into_body();
        RawResponse::from_head_and_materialized_body(head, body)
            .expect("Response::into_body returns a materialized body")
            .with_network_request_extra_info(network_request_extra_info)
    }
}

#[derive(Debug)]
pub struct RawResponse {
    pub final_url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    body: ResponseBody,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<RedirectInfo>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_extra_info: Option<NetworkRequestExtraInfo>,
}

impl Clone for RawResponse {
    fn clone(&self) -> Self {
        Self {
            final_url: self.final_url.clone(),
            status: self.status,
            headers: self.headers.clone(),
            body: self
                .body
                .clone_materialized()
                .expect("RawResponse body should remain materialized"),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.clone(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
            network_request_extra_info: self.network_request_extra_info.clone(),
        }
    }
}

impl RawResponse {
    pub fn with_network_request_extra_info(
        mut self,
        network_request_extra_info: Option<NetworkRequestExtraInfo>,
    ) -> Self {
        self.network_request_extra_info = network_request_extra_info;
        self
    }

    pub fn network_request_extra_info(&self) -> Option<&NetworkRequestExtraInfo> {
        self.network_request_extra_info.as_ref()
    }

    pub fn body_bytes(&self) -> &[u8] {
        self.body
            .as_materialized_bytes()
            .expect("RawResponse body should remain materialized")
    }

    /// Clones the complete materialized byte payload for compatibility callers
    /// whose public contract requires an owned full-body buffer.
    pub fn clone_body_bytes(&self) -> Vec<u8> {
        self.body_bytes().to_vec()
    }

    pub fn materialized_body(&self) -> ResponseBody {
        self.body
            .clone_materialized()
            .expect("RawResponse body should remain materialized")
    }

    pub fn head(&self) -> ResponseHead {
        ResponseHead {
            final_url: self.final_url.clone(),
            status: self.status,
            headers: self.headers.clone(),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.clone(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        }
    }

    pub fn from_head_and_body(head: ResponseHead, body: Vec<u8>) -> Self {
        Self::from_head_and_materialized_body(head, ResponseBody::materialized_bytes(body))
            .expect("materialized bytes body should build a RawResponse")
    }

    pub fn from_head_and_materialized_body(head: ResponseHead, body: ResponseBody) -> Result<Self> {
        let body = body.try_into_materialized_bytes().map_err(|_| {
            anyhow!("cannot build materialized RawResponse from streaming response body")
        })?;
        Ok(Self {
            final_url: head.final_url,
            status: head.status,
            headers: head.headers,
            body: ResponseBody::materialized_bytes(body),
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            redirected: head.redirected,
            redirect_chain: head.redirect_chain,
            from_cache: head.from_cache,
            negotiated_http_version: head.negotiated_http_version,
            network_request_extra_info: None,
        })
    }

    pub async fn from_head_and_body_source(head: ResponseHead, body: ResponseBody) -> Result<Self> {
        let body = body.into_materialized_bytes().await?;
        Ok(Self {
            final_url: head.final_url,
            status: head.status,
            headers: head.headers,
            body: ResponseBody::materialized_bytes(body),
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            redirected: head.redirected,
            redirect_chain: head.redirect_chain,
            from_cache: head.from_cache,
            negotiated_http_version: head.negotiated_http_version,
            network_request_extra_info: None,
        })
    }

    pub fn into_parts(self) -> (ResponseHead, ResponseBody) {
        let head = ResponseHead {
            final_url: self.final_url,
            status: self.status,
            headers: self.headers,
            request_cookie_report: self.request_cookie_report,
            cookie_set_reports: self.cookie_set_reports,
            redirected: self.redirected,
            redirect_chain: self.redirect_chain,
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        };
        (head, self.body)
    }

    pub fn into_body(self) -> (ResponseHead, ResponseBody) {
        self.into_parts()
    }

    pub fn into_lossy_materialized_text_response(self) -> Response {
        let network_request_extra_info = self.network_request_extra_info.clone();
        let (head, body) = self.into_body();
        Response::from_head_and_materialized_body(head, body)
            .expect("RawResponse::into_body returns a materialized body")
            .with_network_request_extra_info(network_request_extra_info)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRequestExtraInfo {
    pub headers: Vec<(String, String)>,
    pub cookie_report: StoredCookieQueryReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkResponseExtraInfo {
    pub request_extra_info: NetworkRequestExtraInfo,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectInfo {
    pub from_url: Url,
    pub to_url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Whether this redirect came from an observable HTTP response header block.
    ///
    /// Cached and browser-internal redirects do not have Network ExtraInfo even
    /// though they still participate in the public redirect chain.
    pub network_extra_info_available: bool,
    pub request_extra_info: Option<NetworkRequestExtraInfo>,
    pub response_extra_info: Option<NetworkResponseExtraInfo>,
    pub redirect_has_extra_info: bool,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
}
