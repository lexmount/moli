use std::{num::NonZeroU32, path::PathBuf};

use cidr::AnyIpCidr;
use moli_browser_profile::{BrowserIdentityProfile, DEFAULT_ACCEPT_LANGUAGE};
use moli_curl::CurlTlsConfig;

use crate::WebBotAuthSigner;

#[derive(Debug, Clone, PartialEq)]
pub struct FetchConfig {
    browser_identity: BrowserIdentityProfile,
    default_request_headers: Vec<(String, String)>,
    request_timeout_ms: u64,
    http_connect_timeout_ms: Option<u64>,
    obey_robots: bool,
    http_proxy: Option<String>,
    http_no_proxy: Option<String>,
    http_cache_dir: Option<String>,
    http_cache_max_bytes: Option<u64>,
    http_host_resolve: Vec<String>,
    proxy_bearer_token: Option<String>,
    http_max_concurrent: Option<NonZeroU32>,
    // Scheduler cap for simultaneously active transfers to one origin. This is
    // intentionally separate from the libcurl per-host connection-pool cap so
    // HTTP/2 can use multiple streams without being limited by the HTTP/1
    // socket default.
    http_max_host_open: Option<NonZeroU32>,
    // Transport cap passed to libcurl's per-host connection pool. When unset,
    // Moli uses Chromium's HTTP/1-style default of six connections per
    // host/group.
    http_max_host_connections: Option<u8>,
    // Transport cap for total cached/open connections across hosts.
    http_max_total_connections: Option<u16>,
    // HTTP/2 stream cap; this is separate from HTTP/1 connection count.
    http2_max_concurrent_streams: Option<u16>,
    http_max_response_size: Option<usize>,
    block_private_networks: bool,
    block_cidrs: Vec<AnyIpCidr>,
    tls: CurlTlsConfig,
    web_bot_auth: Option<WebBotAuthSigner>,
}

impl FetchConfig {
    pub const DEFAULT_USER_AGENT: &str = moli_browser_profile::DEFAULT_USER_AGENT;
    /// Chromium's normal socket pool uses six connections per host/group for
    /// HTTP/1-style connection pooling. This is a transport connection cap, not
    /// a runtime active-transfer cap and not an HTTP/2 stream cap.
    pub const DEFAULT_HTTP_MAX_HOST_CONNECTIONS: u8 = 6;

    pub fn user_agent(&self) -> &str {
        self.browser_identity.user_agent()
    }

    pub fn set_user_agent(&mut self, user_agent: impl Into<String>) {
        self.browser_identity =
            BrowserIdentityProfile::new(user_agent, self.browser_identity.accept_language());
    }

    pub fn set_browser_identity(&mut self, browser_identity: BrowserIdentityProfile) {
        self.browser_identity = browser_identity;
    }

    pub fn set_user_agent_suffix(&mut self, suffix: impl AsRef<str>) {
        self.set_user_agent(format!("{} {}", Self::DEFAULT_USER_AGENT, suffix.as_ref()));
    }

    pub fn browser_identity(&self) -> &BrowserIdentityProfile {
        &self.browser_identity
    }

    pub fn default_request_headers(&self) -> &[(String, String)] {
        &self.default_request_headers
    }

    pub fn set_default_request_headers(&mut self, headers: Vec<(String, String)>) {
        self.default_request_headers = headers;
        self.refresh_identity_accept_language();
    }

    pub fn push_default_request_header(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.default_request_headers
            .push((name.into(), value.into()));
        self.refresh_identity_accept_language();
    }

    pub fn request_timeout_ms(&self) -> u64 {
        self.request_timeout_ms
    }

    pub fn set_request_timeout_ms(&mut self, request_timeout_ms: u64) {
        self.request_timeout_ms = request_timeout_ms;
    }

    pub fn set_connect_timeout_ms(&mut self, http_connect_timeout_ms: Option<u64>) {
        self.http_connect_timeout_ms = http_connect_timeout_ms;
    }

    pub fn set_obey_robots(&mut self, obey_robots: bool) {
        self.obey_robots = obey_robots;
    }

    pub fn set_proxy_options(
        &mut self,
        http_proxy: Option<String>,
        proxy_bearer_token: Option<String>,
    ) {
        self.http_proxy = http_proxy;
        self.proxy_bearer_token = proxy_bearer_token;
    }

    pub fn set_connection_limits(
        &mut self,
        http_max_concurrent: Option<NonZeroU32>,
        http_max_host_open: Option<NonZeroU32>,
        http_max_response_size: Option<usize>,
    ) {
        // These are fetch-runtime scheduler limits. In particular,
        // `http_max_host_open` limits active work per origin and does not
        // configure libcurl's connection pool.
        self.http_max_concurrent = http_max_concurrent;
        self.http_max_host_open = http_max_host_open;
        self.http_max_response_size = http_max_response_size;
    }

    pub fn set_transport_connection_limits(
        &mut self,
        http_max_host_connections: Option<u8>,
        http_max_total_connections: Option<u16>,
        http2_max_concurrent_streams: Option<u16>,
    ) {
        // These are transport-level limits handed to curl. Keep them out of the
        // runtime scheduler so a Chromium-like HTTP/1 connection default does
        // not accidentally throttle HTTP/2 streams or queued browser work.
        self.http_max_host_connections = http_max_host_connections;
        self.http_max_total_connections = http_max_total_connections;
        self.http2_max_concurrent_streams = http2_max_concurrent_streams;
    }

    pub fn tls_verify_host(&self) -> bool {
        self.tls.verify
    }

    pub fn set_tls_verify_host(&mut self, tls_verify_host: bool) {
        self.tls.verify = tls_verify_host;
    }

    pub fn tls_config(&self) -> &CurlTlsConfig {
        &self.tls
    }

    /// Configure TLS trust and mutual-TLS credentials for this fetch runtime,
    /// including navigations and subresources. Client identities are used only
    /// when the request's credentials mode allows them for the current URL;
    /// CA trust applies regardless of credentials mode.
    pub fn set_tls_credentials(
        &mut self,
        ca_cert: Option<PathBuf>,
        client_cert: Option<PathBuf>,
        client_key: Option<PathBuf>,
        client_cert_password: Option<String>,
    ) {
        self.tls.ca_cert = ca_cert;
        self.tls.client_cert = client_cert;
        self.tls.client_key = client_key;
        self.tls.client_cert_password = client_cert_password;
    }

    pub fn ca_cert(&self) -> Option<&std::path::Path> {
        self.tls.ca_cert.as_deref()
    }

    pub fn client_cert(&self) -> Option<&std::path::Path> {
        self.tls.client_cert.as_deref()
    }

    pub fn client_key(&self) -> Option<&std::path::Path> {
        self.tls.client_key.as_deref()
    }

    pub fn client_cert_password(&self) -> Option<&str> {
        self.tls.client_cert_password.as_deref()
    }

    pub fn web_bot_auth(&self) -> Option<&WebBotAuthSigner> {
        self.web_bot_auth.as_ref()
    }

    pub fn set_web_bot_auth(&mut self, web_bot_auth: Option<WebBotAuthSigner>) {
        self.web_bot_auth = web_bot_auth;
    }

    pub fn set_http_proxy(&mut self, http_proxy: Option<String>) {
        self.http_proxy = http_proxy;
    }

    pub fn set_http_no_proxy(&mut self, http_no_proxy: Option<String>) {
        self.http_no_proxy = http_no_proxy;
    }

    pub fn set_http_host_resolve(&mut self, entries: Vec<String>) {
        self.http_host_resolve = entries;
    }

    pub fn set_http_cache_dir(&mut self, http_cache_dir: Option<String>) {
        self.http_cache_dir = http_cache_dir;
    }

    pub fn set_http_cache_max_bytes(&mut self, http_cache_max_bytes: Option<u64>) {
        self.http_cache_max_bytes = http_cache_max_bytes;
    }

    pub fn set_network_blocking(
        &mut self,
        block_private_networks: bool,
        block_cidrs: Vec<AnyIpCidr>,
    ) {
        self.block_private_networks = block_private_networks;
        self.block_cidrs = block_cidrs;
    }

    pub fn http_proxy(&self) -> Option<&str> {
        self.http_proxy.as_deref()
    }

    pub fn http_no_proxy(&self) -> Option<&str> {
        self.http_no_proxy.as_deref()
    }

    pub fn http_host_resolve(&self) -> &[String] {
        &self.http_host_resolve
    }

    pub fn http_cache_dir(&self) -> Option<&str> {
        self.http_cache_dir.as_deref()
    }

    pub fn http_cache_max_bytes(&self) -> Option<u64> {
        self.http_cache_max_bytes
    }

    pub fn proxy_bearer_token(&self) -> Option<&str> {
        self.proxy_bearer_token.as_deref()
    }

    pub fn http_connect_timeout_ms(&self) -> Option<u64> {
        self.http_connect_timeout_ms
    }

    pub fn obey_robots(&self) -> bool {
        self.obey_robots
    }

    pub fn http_max_concurrent(&self) -> Option<NonZeroU32> {
        self.http_max_concurrent
    }

    pub fn http_max_host_open(&self) -> Option<NonZeroU32> {
        self.http_max_host_open
    }

    /// Effective libcurl per-host connection cap.
    ///
    /// The default only applies to the transport connection pool. It must not be
    /// reused as a scheduler per-origin active-transfer cap, because that would
    /// also throttle HTTP/2 streams to the HTTP/1 connection default.
    pub fn effective_http_max_host_connections(&self) -> Option<u8> {
        match self.http_max_host_connections {
            Some(0) => None,
            Some(value) => Some(value),
            None => Some(Self::DEFAULT_HTTP_MAX_HOST_CONNECTIONS),
        }
    }

    pub fn http_max_host_connections(&self) -> Option<u8> {
        self.http_max_host_connections
    }

    pub fn http_max_total_connections(&self) -> Option<u16> {
        self.http_max_total_connections
    }

    pub fn http2_max_concurrent_streams(&self) -> Option<u16> {
        self.http2_max_concurrent_streams
    }

    pub fn http_max_response_size(&self) -> Option<usize> {
        self.http_max_response_size
    }

    pub fn block_private_networks(&self) -> bool {
        self.block_private_networks
    }

    pub fn block_cidrs(&self) -> &[AnyIpCidr] {
        &self.block_cidrs
    }
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            browser_identity: BrowserIdentityProfile::default(),
            default_request_headers: Vec::new(),
            request_timeout_ms: 30_000,
            http_connect_timeout_ms: None,
            obey_robots: false,
            http_proxy: None,
            http_no_proxy: None,
            http_cache_dir: None,
            http_cache_max_bytes: Some(100 * 1024 * 1024),
            http_host_resolve: Vec::new(),
            proxy_bearer_token: None,
            http_max_concurrent: None,
            http_max_host_open: None,
            http_max_host_connections: None,
            http_max_total_connections: None,
            http2_max_concurrent_streams: None,
            http_max_response_size: None,
            block_private_networks: false,
            block_cidrs: Vec::new(),
            tls: CurlTlsConfig::default(),
            web_bot_auth: None,
        }
    }
}

impl FetchConfig {
    fn refresh_identity_accept_language(&mut self) {
        let accept_language = self
            .default_request_headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("accept-language"))
            .map(|(_, value)| value.as_str())
            .unwrap_or(DEFAULT_ACCEPT_LANGUAGE)
            .to_owned();
        self.browser_identity = self.browser_identity.with_accept_language(accept_language);
    }
}
