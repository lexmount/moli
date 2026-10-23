use std::{collections::HashSet, fmt};

use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketUrlError {
    Invalid,
    SchemeNormalizationFailed,
    DisallowedScheme(String),
    Fragment,
}

impl fmt::Display for WebSocketUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => write!(f, "the provided URL is invalid"),
            Self::SchemeNormalizationFailed => write!(f, "failed to normalize URL scheme"),
            Self::DisallowedScheme(scheme) => {
                write!(f, "URL scheme `{scheme}` is not allowed")
            }
            Self::Fragment => write!(f, "URL contains a fragment identifier"),
        }
    }
}

impl std::error::Error for WebSocketUrlError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketSubprotocolError {
    Invalid(String),
    Duplicate(String),
}

impl fmt::Display for WebSocketSubprotocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(protocol) => write!(f, "subprotocol `{protocol}` is invalid"),
            Self::Duplicate(protocol) => write!(f, "subprotocol `{protocol}` is duplicated"),
        }
    }
}

impl std::error::Error for WebSocketSubprotocolError {}

pub fn normalize_websocket_url(base_url: &Url, input: &str) -> Result<Url, WebSocketUrlError> {
    let mut url = Url::parse(input)
        .or_else(|_| base_url.join(input))
        .map_err(|_| WebSocketUrlError::Invalid)?;
    match url.scheme() {
        "http" => {
            url.set_scheme("ws")
                .map_err(|_| WebSocketUrlError::SchemeNormalizationFailed)?;
        }
        "https" => {
            url.set_scheme("wss")
                .map_err(|_| WebSocketUrlError::SchemeNormalizationFailed)?;
        }
        "ws" | "wss" => {}
        scheme => return Err(WebSocketUrlError::DisallowedScheme(scheme.to_owned())),
    }
    if url.fragment().is_some() {
        return Err(WebSocketUrlError::Fragment);
    }
    Ok(url)
}

pub fn validate_subprotocols(protocols: &[String]) -> Result<(), WebSocketSubprotocolError> {
    let mut seen = HashSet::new();
    for protocol in protocols {
        if !is_valid_subprotocol(protocol) {
            return Err(WebSocketSubprotocolError::Invalid(protocol.clone()));
        }
        if !seen.insert(protocol.to_ascii_lowercase()) {
            return Err(WebSocketSubprotocolError::Duplicate(protocol.clone()));
        }
    }
    Ok(())
}

pub fn is_valid_subprotocol(protocol: &str) -> bool {
    !protocol.is_empty()
        && protocol.bytes().all(|byte| {
            matches!(byte, b'!'..=b'~')
                && !matches!(
                    byte,
                    b'"' | b'(' | b')' | b',' | b'/' | b':'..=b'@' | b'['..=b']' | b'{' | b'}'
                )
        })
}

pub fn websocket_url_is_potentially_trustworthy(url: &Url) -> bool {
    moli_url::is_potentially_trustworthy_url(url)
}

pub fn is_valid_close_code(code: u16) -> bool {
    code == 1000 || (3000..=4999).contains(&code)
}

pub fn is_valid_close_reason(reason: &str) -> bool {
    reason.len() <= 123
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketCloseValidationError {
    InvalidCode,
    ReasonTooLong,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketCloseRequest {
    pub code: Option<u16>,
    pub reason: String,
}

pub fn default_close_code_for_reason(close_code: Option<u16>, reason: &str) -> Option<u16> {
    if close_code.is_none() && !reason.is_empty() {
        Some(1000)
    } else {
        close_code
    }
}

pub fn validate_websocket_close_request(
    code: Option<u16>,
    reason: String,
) -> Result<WebSocketCloseRequest, WebSocketCloseValidationError> {
    if code.is_some_and(|code| !is_valid_close_code(code)) {
        return Err(WebSocketCloseValidationError::InvalidCode);
    }
    if !is_valid_close_reason(&reason) {
        return Err(WebSocketCloseValidationError::ReasonTooLong);
    }
    Ok(WebSocketCloseRequest { code, reason })
}

pub fn close_info_code_from_number(value: f64) -> Result<u16, WebSocketCloseValidationError> {
    if !value.is_finite() {
        return Err(WebSocketCloseValidationError::InvalidCode);
    }
    let code = value.trunc().clamp(0.0, u16::MAX as f64) as u16;
    if is_valid_close_code(code) {
        Ok(code)
    } else {
        Err(WebSocketCloseValidationError::InvalidCode)
    }
}

pub fn normalize_websocket_close_info(
    code: Option<u16>,
    reason: String,
) -> Result<WebSocketCloseRequest, WebSocketCloseValidationError> {
    validate_websocket_close_request(default_close_code_for_reason(code, &reason), reason)
}
