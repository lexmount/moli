//! Shared libcurl multi scheduler for Moli network requests.

mod dns_adapter;
mod http;
mod runtime;
mod tls;
pub mod websocket;

pub use dns_adapter::CurlDnsResolution;
pub use http::{CurlHttpSender, CurlMultiCompletion, CurlMultiJob, CurlOriginKey, CurlSubmitError};
pub use runtime::{CurlMultiRuntime, CurlMultiRuntimeConfig, CurlTransferId};
pub use tls::CurlTlsConfig;
