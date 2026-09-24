use std::{error::Error, fmt};

use url::Url;

/// Network policy failures produced by the navigation request loaders.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationNetworkErrorKind {
    InternetDisconnected,
}

impl NavigationNetworkErrorKind {
    pub fn error_text(self) -> &'static str {
        match self {
            Self::InternetDisconnected => "net::ERR_INTERNET_DISCONNECTED",
        }
    }
}

/// A navigation request rejected by emulated offline network policy.
///
/// Transport failures retain `moli_fetch::NetworkFetchFailureContext` and use
/// their existing handling paths; this cause currently represents offline
/// enforcement only.
///
/// This is an error cause rather than display context, so callers can recover
/// its request identity through `anyhow::Error::downcast_ref` after adding
/// diagnostic context along the navigation pipeline.
#[derive(Debug)]
pub struct NavigationNetworkError {
    pub kind: NavigationNetworkErrorKind,
    pub unreachable_url: Url,
    pub request_method: String,
    pub request_headers: moli_fetch::RequestHeaders,
}

impl fmt::Display for NavigationNetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.error_text())
    }
}

impl Error for NavigationNetworkError {}

/// A navigation request rejected by `Network.setBlockedURLs`.
///
/// The protocol boundary preserves its canonical error text independently of
/// diagnostic context on other navigation failures.
#[derive(Debug)]
pub struct NavigationRequestBlocked;

impl fmt::Display for NavigationRequestBlocked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("net::ERR_BLOCKED_BY_CLIENT")
    }
}

impl Error for NavigationRequestBlocked {}
