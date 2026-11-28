//! Shared browser URL capability routing.
//!
//! URL parsing answers whether a URL is syntactically valid. It does not grant
//! a browser subsystem permission to load that URL. This crate keeps those two
//! decisions separate and makes each caller name the browser operation it is
//! performing.
//!
//! In particular, the HTTP network transport accepts only HTTP(S). A future
//! local-file loader may consume [`BrowserUrlRoute::LocalFile`], but a file URL
//! must never be handed to libcurl as an HTTP request.

mod error;
mod route;
mod scheme;

pub use error::{UrlPolicyError, UrlPolicyReason, UrlRequestContext};
pub use route::{
    BrowserUrlRoute, LocalFileNavigationAccess, LocalFilePolicy, ensure_http_network_transport_url,
    route_fetch_url, route_navigation_url, route_service_worker_url, route_websocket_url,
    route_xml_http_request_url,
};
pub use scheme::BrowserUrlScheme;

#[cfg(test)]
mod tests;
