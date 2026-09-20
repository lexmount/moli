//! Shared libcurl multi scheduler for Moli network requests.

mod dns_adapter;
mod host_resolve;
mod http;
mod network_policy;
mod proxy;
mod request_headers;
mod runtime;
mod tls;
pub mod websocket;

pub use dns_adapter::CurlDnsResolution;
pub use host_resolve::{HostResolveOverrides, validate_http_host_resolve_entries};
pub use http::{CurlHttpSender, CurlMultiCompletion, CurlMultiJob, CurlOriginKey, CurlSubmitError};
pub use network_policy::NetworkAddressPolicy;
pub use proxy::{
    ConnectionDnsEndpoint, ConnectionEndpointRole, ProxyRoute, ProxyScheme, ProxyTargetResolution,
    SelectedProxy, select_proxy_route, select_proxy_route_with_env,
};
pub use request_headers::RequestHeaderList;
pub use runtime::{CurlMultiRuntime, CurlMultiRuntimeConfig, CurlTransferId};
pub use tls::CurlTlsConfig;
