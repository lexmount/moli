//! Shared libcurl multi scheduler for Moli network requests.

mod dns_adapter;
mod runtime;
mod tls;
pub mod websocket;

pub use dns_adapter::CurlDnsResolution;
pub use runtime::{
    CurlHttpSender, CurlMultiCompletion, CurlMultiJob, CurlMultiRuntime, CurlMultiRuntimeConfig,
    CurlOriginKey, CurlSubmitError, CurlTransferId,
};
pub use tls::CurlTlsConfig;
