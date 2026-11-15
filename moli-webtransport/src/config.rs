use std::time::Duration;

use http::HeaderMap;
use moli_curl::{CurlTlsConfig, NetworkAddressPolicy};
use moli_dns_resolver::DnsCachePartition;
use url::Url;

/// Browser-context transport policy, captured independently of V8 callbacks.
#[derive(Clone, Debug)]
pub struct Config {
    pub tls: CurlTlsConfig,
    pub address_policy: NetworkAddressPolicy,
    pub host_resolve: Vec<String>,
    pub proxy: Option<String>,
    pub no_proxy: Option<String>,
    pub dns_partition: DnsCachePartition,
    /// Bounds DNS, TLS and HTTP/3 session establishment, not session lifetime.
    pub connect_timeout: Option<Duration>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tls: CurlTlsConfig::default(),
            address_policy: NetworkAddressPolicy::default(),
            host_resolve: Vec::new(),
            proxy: None,
            no_proxy: None,
            dns_partition: DnsCachePartition::fresh(),
            connect_timeout: Some(Duration::from_secs(30)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum CongestionControl {
    #[default]
    Default,
    Throughput,
    LowLatency,
}

/// An owned certificate fingerprint captured after WebIDL conversion.
#[derive(Clone, Debug)]
pub struct CertificateHash {
    pub algorithm: String,
    pub value: Vec<u8>,
}

/// Typed session inputs after the binding has applied WebIDL and CSP policy.
/// A nonempty certificate hash list replaces Web PKI for this session only.
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub url: Url,
    pub origin: String,
    pub protocols: Vec<String>,
    pub headers: HeaderMap,
    pub congestion_control: CongestionControl,
    pub server_certificate_hashes: Vec<CertificateHash>,
}

impl ConnectOptions {
    pub fn new(url: Url, origin: String) -> Self {
        Self {
            url,
            origin,
            protocols: Vec::new(),
            headers: HeaderMap::new(),
            congestion_control: CongestionControl::Default,
            server_certificate_hashes: Vec::new(),
        }
    }
}
