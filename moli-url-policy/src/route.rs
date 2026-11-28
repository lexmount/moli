use moli_url::is_about_blank;
use url::Url;

use crate::{BrowserUrlScheme, UrlPolicyError, UrlPolicyReason, UrlRequestContext};

/// The loader family selected after a browser capability check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserUrlRoute {
    HttpNetwork,
    LocalData,
    LocalBlob,
    EmptyDocument,
    LocalFile,
    WebSocket,
}

/// Startup restrictions on file URL loading and automation-selected host paths.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LocalFilePolicy {
    #[default]
    BrowserCompatible,
    BlockFileUrls,
    BlockHostFileAccess,
}

impl LocalFilePolicy {
    pub fn blocks_file_urls(self) -> bool {
        self != Self::BrowserCompatible
    }

    pub fn check_host_file_access(self) -> std::io::Result<()> {
        if self == Self::BlockHostFileAccess {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Host file access is disabled by browser policy",
            ));
        }
        Ok(())
    }
}

/// File navigation is a browser-granted capability, not a consequence of URL
/// parsing. Hosted browser entry points should use [`Self::Denied`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LocalFileNavigationAccess {
    #[default]
    Denied,
    BrowserGranted,
}

/// Route a Window or Worker Fetch request.
pub fn route_fetch_url(url: &Url) -> Result<BrowserUrlRoute, UrlPolicyError> {
    route_script_request_url(url, UrlRequestContext::Fetch)
}

/// Route a Window or Worker XMLHttpRequest request.
pub fn route_xml_http_request_url(url: &Url) -> Result<BrowserUrlRoute, UrlPolicyError> {
    route_script_request_url(url, UrlRequestContext::XmlHttpRequest)
}

fn route_script_request_url(
    url: &Url,
    context: UrlRequestContext,
) -> Result<BrowserUrlRoute, UrlPolicyError> {
    match BrowserUrlScheme::from_url(url) {
        BrowserUrlScheme::Http | BrowserUrlScheme::Https => Ok(BrowserUrlRoute::HttpNetwork),
        BrowserUrlScheme::Data => Ok(BrowserUrlRoute::LocalData),
        BrowserUrlScheme::Blob => Ok(BrowserUrlRoute::LocalBlob),
        _ => Err(UrlPolicyError::new(
            url,
            context,
            UrlPolicyReason::UnsupportedScheme,
        )),
    }
}

/// Route a document navigation.
///
/// A granted file route still must be consumed by a dedicated file loader. The
/// HTTP transport rejects it independently.
pub fn route_navigation_url(
    url: &Url,
    local_file_access: LocalFileNavigationAccess,
) -> Result<BrowserUrlRoute, UrlPolicyError> {
    match BrowserUrlScheme::from_url(url) {
        BrowserUrlScheme::Http | BrowserUrlScheme::Https => Ok(BrowserUrlRoute::HttpNetwork),
        BrowserUrlScheme::Data => Ok(BrowserUrlRoute::LocalData),
        BrowserUrlScheme::Blob => Ok(BrowserUrlRoute::LocalBlob),
        BrowserUrlScheme::About if is_about_blank(url) => Ok(BrowserUrlRoute::EmptyDocument),
        BrowserUrlScheme::About => Err(UrlPolicyError::new(
            url,
            UrlRequestContext::Navigation,
            UrlPolicyReason::UnsupportedAboutUrl,
        )),
        BrowserUrlScheme::File
            if local_file_access == LocalFileNavigationAccess::BrowserGranted =>
        {
            Ok(BrowserUrlRoute::LocalFile)
        }
        BrowserUrlScheme::File => Err(UrlPolicyError::new(
            url,
            UrlRequestContext::Navigation,
            UrlPolicyReason::LocalFileCapabilityRequired,
        )),
        _ => Err(UrlPolicyError::new(
            url,
            UrlRequestContext::Navigation,
            UrlPolicyReason::UnsupportedScheme,
        )),
    }
}

/// Final safety boundary for requests entering the libcurl HTTP transport.
pub fn ensure_http_network_transport_url(url: &Url) -> Result<(), UrlPolicyError> {
    if BrowserUrlScheme::from_url(url).uses_http_network_transport() {
        Ok(())
    } else {
        Err(UrlPolicyError::new(
            url,
            UrlRequestContext::HttpNetworkTransport,
            UrlPolicyReason::NonHttpNetworkScheme,
        ))
    }
}

pub fn route_service_worker_url(url: &Url) -> Result<BrowserUrlRoute, UrlPolicyError> {
    if BrowserUrlScheme::from_url(url).supports_service_worker() {
        Ok(BrowserUrlRoute::HttpNetwork)
    } else {
        Err(UrlPolicyError::new(
            url,
            UrlRequestContext::ServiceWorker,
            UrlPolicyReason::UnsupportedScheme,
        ))
    }
}

pub fn route_websocket_url(url: &Url) -> Result<BrowserUrlRoute, UrlPolicyError> {
    if matches!(
        BrowserUrlScheme::from_url(url),
        BrowserUrlScheme::WebSocket | BrowserUrlScheme::SecureWebSocket
    ) {
        Ok(BrowserUrlRoute::WebSocket)
    } else {
        Err(UrlPolicyError::new(
            url,
            UrlRequestContext::WebSocket,
            UrlPolicyReason::NonWebSocketScheme,
        ))
    }
}
