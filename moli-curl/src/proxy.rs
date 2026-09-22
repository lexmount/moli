use std::net::IpAddr;

use anyhow::{Context, Result, bail};
use cidr::AnyIpCidr;
use moli_dns_resolver::DnsTarget;
use url::{Host, Position, Url};

use crate::HostResolveOverrides;

/// How an accepted proxy resolves the request target hostname.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyTargetResolution {
    /// The target hostname is sent to the proxy without a local DNS lookup.
    Proxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionEndpointRole {
    RequestTarget,
    Proxy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionDnsEndpoint {
    target: DnsTarget,
    role: ConnectionEndpointRole,
}

impl ConnectionDnsEndpoint {
    pub fn target(&self) -> &DnsTarget {
        &self.target
    }

    pub fn role(&self) -> ConnectionEndpointRole {
        self.role
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyScheme {
    Http,
    Https,
    Socks5h,
    Socks4a,
}

impl ProxyScheme {
    pub fn uses_http_headers(self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }
}

/// Parsed proxy selected for a single request target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedProxy {
    /// Proxy URL as configured by the caller, retained for diagnostics.
    url: Url,
    /// Equivalent remote-DNS URL understood by libcurl.
    curl_url: String,
    scheme: ProxyScheme,
    endpoint_port: u16,
}

impl SelectedProxy {
    pub fn parse(raw: &str) -> Result<Self> {
        let normalized = if raw.contains("://") {
            raw.to_owned()
        } else {
            format!("http://{raw}")
        };
        let url = Url::parse(&normalized)
            .with_context(|| format!("failed to parse proxy URL `{raw}`"))?;
        // Chromium's public `socks`/`socks5` spelling always sends the target
        // hostname to the proxy. libcurl assigns local-DNS semantics to
        // `socks5`, so use its explicit `socks5h` spelling internally. The
        // analogous SOCKS4 hostname form is `socks4a`. Keeping `url` unchanged
        // preserves useful diagnostics while `curl_url` carries this semantic
        // translation to the transport.
        let (scheme, curl_scheme) = match url.scheme() {
            "http" => (ProxyScheme::Http, "http"),
            "https" => (ProxyScheme::Https, "https"),
            "socks" | "socks5" | "socks5h" => (ProxyScheme::Socks5h, "socks5h"),
            "socks4" | "socks4a" => (ProxyScheme::Socks4a, "socks4a"),
            scheme => bail!(
                "unsupported proxy scheme `{scheme}`; expected http, https, socks, socks5, socks5h, socks4, or socks4a"
            ),
        };
        if url.host().is_none() {
            bail!("proxy URL `{raw}` is missing a host");
        }
        // libcurl's parse_proxy() defaults to 443 for HTTPS and 1080 for
        // HTTP/SOCKS (see CURLOPT_PROXYPORT). Read the explicit port from the
        // original authority because `url::Url` deliberately drops explicit
        // default ports such as `:80` and `:443`.
        let default_port = match scheme {
            ProxyScheme::Https => 443,
            _ => 1080,
        };
        let endpoint_port = explicit_proxy_port(&normalized)?.unwrap_or(default_port);
        let mut curl_url = url.clone();
        curl_url
            .set_scheme(curl_scheme)
            .map_err(|()| anyhow::anyhow!("failed to normalize proxy URL `{raw}` for curl"))?;
        curl_url
            .set_port(None)
            .map_err(|()| anyhow::anyhow!("failed to normalize proxy URL `{raw}` for curl"))?;
        // Always spell out the chosen port. Apart from preserving explicit
        // `:80`/`:443`, this makes CURLOPT_RESOLVE and CURLOPT_PROXY name the
        // exact same connection endpoint and prevents curl from applying a
        // different implicit port after Moli has pinned DNS.
        let curl_url = format!(
            "{}:{endpoint_port}{}",
            &curl_url[..Position::AfterHost],
            &curl_url[Position::AfterHost..]
        );
        Ok(Self {
            url,
            curl_url,
            scheme,
            endpoint_port,
        })
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    /// Proxy URL passed to libcurl after applying Moli's remote-DNS semantics.
    pub fn curl_url(&self) -> &str {
        &self.curl_url
    }

    pub fn scheme(&self) -> ProxyScheme {
        self.scheme
    }

    pub fn endpoint_host(&self) -> &str {
        self.url
            .host_str()
            .expect("selected proxy was validated with a host")
    }

    pub fn endpoint_ip(&self) -> Option<IpAddr> {
        match self
            .url
            .host()
            .expect("selected proxy was validated with a host")
        {
            Host::Ipv4(address) => Some(address.into()),
            Host::Ipv6(address) => Some(address.into()),
            Host::Domain(_) => None,
        }
    }

    pub fn endpoint_port(&self) -> u16 {
        self.endpoint_port
    }

    pub fn target_resolution(&self) -> ProxyTargetResolution {
        ProxyTargetResolution::Proxy
    }
}

fn explicit_proxy_port(raw_url: &str) -> Result<Option<u16>> {
    let (_, remainder) = raw_url
        .split_once("://")
        .expect("proxy URL is normalized with a scheme");
    let authority = remainder
        .split(['/', '?', '#'])
        .next()
        .expect("split always yields the authority");
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host_and_port)| host_and_port);

    let port = if let Some(bracketed) = host_and_port.strip_prefix('[') {
        let closing_bracket = bracketed
            .find(']')
            .expect("proxy URL parser validated the IPv6 host");
        bracketed[closing_bracket + 1..].strip_prefix(':')
    } else {
        host_and_port.rsplit_once(':').map(|(_, port)| port)
    };

    port.map(|port| {
        port.parse::<u16>()
            .with_context(|| format!("proxy URL `{raw_url}` has an invalid port"))
    })
    .transpose()
}

/// Concrete route selected exactly once for a request target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyRoute {
    Direct,
    Proxy(SelectedProxy),
}

impl ProxyRoute {
    pub fn from_proxy_url(raw: &str) -> Result<Self> {
        Ok(Self::Proxy(SelectedProxy::parse(raw)?))
    }

    pub fn is_proxy(&self) -> bool {
        matches!(self, Self::Proxy(_))
    }

    pub fn proxy(&self) -> Option<&SelectedProxy> {
        match self {
            Self::Direct => None,
            Self::Proxy(proxy) => Some(proxy),
        }
    }

    /// Returns the one hostname Moli must resolve locally for this route.
    ///
    /// IP literals and fixed host-resolve entries need no lookup. A direct
    /// route resolves the request target; a remote-DNS proxy route resolves
    /// only the proxy endpoint.
    pub fn connection_dns_endpoint(
        &self,
        request_url: &Url,
        host_resolve: &HostResolveOverrides,
    ) -> Result<Option<ConnectionDnsEndpoint>> {
        let request_host = request_url
            .host_str()
            .with_context(|| format!("request URL `{request_url}` is missing a host"))?;
        let request_port = request_url
            .port_or_known_default()
            .with_context(|| format!("request URL `{request_url}` has no port"))?;

        match self {
            Self::Direct => {
                if request_url
                    .host()
                    .is_some_and(|host| matches!(host, Host::Domain(_)))
                    && host_resolve
                        .addresses_for(request_host, request_port)
                        .is_none()
                {
                    return Ok(Some(ConnectionDnsEndpoint {
                        target: DnsTarget::new(request_host, request_port),
                        role: ConnectionEndpointRole::RequestTarget,
                    }));
                }
                Ok(None)
            }
            Self::Proxy(proxy) => {
                // A target override cannot influence a remote-DNS proxy. Fail
                // instead of silently giving the caller a false pinning
                // guarantee. The one exception is when target and proxy are
                // literally the same endpoint, where the entry pins the local
                // proxy connection itself.
                let proxy_is_request_endpoint =
                    proxy.endpoint_host().eq_ignore_ascii_case(request_host)
                        && proxy.endpoint_port() == request_port;
                if !proxy_is_request_endpoint
                    && host_resolve
                        .addresses_for(request_host, request_port)
                        .is_some()
                {
                    bail!(
                        "--http-host-resolve entry for proxied target `{request_host}:{request_port}` would be ignored; remote-DNS proxy `{}` resolves request target hostnames",
                        proxy.url()
                    );
                }
                if proxy.endpoint_ip().is_none()
                    && host_resolve
                        .addresses_for(proxy.endpoint_host(), proxy.endpoint_port())
                        .is_none()
                {
                    return Ok(Some(ConnectionDnsEndpoint {
                        target: DnsTarget::new(proxy.endpoint_host(), proxy.endpoint_port()),
                        role: ConnectionEndpointRole::Proxy,
                    }));
                }
                Ok(None)
            }
        }
    }
}

pub fn select_proxy_route(
    target: &Url,
    configured_proxy: Option<&str>,
    configured_no_proxy: Option<&str>,
) -> Result<ProxyRoute> {
    select_proxy_route_with_env(target, configured_proxy, configured_no_proxy, |name| {
        std::env::var(name).ok()
    })
}

pub fn select_proxy_route_with_env(
    target: &Url,
    configured_proxy: Option<&str>,
    configured_no_proxy: Option<&str>,
    mut env: impl FnMut(&str) -> Option<String>,
) -> Result<ProxyRoute> {
    let proxy = match configured_proxy {
        Some("") => return Ok(ProxyRoute::Direct),
        Some(proxy) => Some(proxy.to_owned()),
        None => env_proxy_for_scheme(target.scheme(), &mut env),
    };
    let Some(proxy) = proxy.filter(|proxy| !proxy.is_empty()) else {
        return Ok(ProxyRoute::Direct);
    };

    let no_proxy = match configured_no_proxy {
        Some(no_proxy) => Some(no_proxy.to_owned()),
        None => env_no_proxy(&mut env),
    };
    if let Some(host) = target.host_str()
        && no_proxy_matches(host, no_proxy.as_deref())
    {
        return Ok(ProxyRoute::Direct);
    }

    ProxyRoute::from_proxy_url(&proxy)
}

fn env_proxy_for_scheme(
    scheme: &str,
    env: &mut impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    let names: &[&str] = match scheme {
        // curl deliberately ignores uppercase HTTP_PROXY because CGI servers
        // commonly expose an attacker-controlled Proxy header under that name.
        "http" | "ws" => &["http_proxy"],
        "https" | "wss" => &["https_proxy", "HTTPS_PROXY"],
        _ => &[],
    };
    for name in names {
        if let Some(value) = env(name).filter(|value| !value.is_empty()) {
            return Some(value);
        }
    }
    for name in ["all_proxy", "ALL_PROXY"] {
        if let Some(value) = env(name).filter(|value| !value.is_empty()) {
            return Some(value);
        }
    }
    None
}

fn env_no_proxy(env: &mut impl FnMut(&str) -> Option<String>) -> Option<String> {
    env("no_proxy")
        .filter(|value| !value.is_empty())
        .or_else(|| env("NO_PROXY").filter(|value| !value.is_empty()))
}

fn no_proxy_matches(host: &str, no_proxy: Option<&str>) -> bool {
    let Some(no_proxy) = no_proxy else {
        return false;
    };
    if host.is_empty() {
        return false;
    }
    // libcurl's Curl_check_noproxy() only receives a hostname: a port suffix
    // is not a request-port constraint. Its wildcard applies only when the
    // entire setting is "*", not when "*" occurs inside a comma-separated list.
    if no_proxy == "*" {
        return true;
    }
    // libcurl removes only one trailing dot from the hostname and at most
    // one dot at each end of a domain token. Removing repeated dots could
    // turn malformed input into a broader proxy bypass rule.
    let host = host.trim_matches(['[', ']']);
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    let host_ip = host.parse::<IpAddr>().ok();
    no_proxy.split(',').any(|token| {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }
        if let Some(host_ip) = host_ip {
            // IPv6 entries in NO_PROXY are bare addresses, without brackets.
            return token.parse::<IpAddr>().is_ok_and(|ip| ip == host_ip)
                || token
                    .parse::<AnyIpCidr>()
                    .is_ok_and(|cidr| cidr.contains(&host_ip));
        }
        let token_host = token.strip_suffix('.').unwrap_or(token);
        let token_host = token_host
            .strip_prefix('.')
            .unwrap_or(token_host)
            .to_ascii_lowercase();
        if token_host.is_empty() {
            return false;
        }
        host == token_host
            || host
                .strip_suffix(&token_host)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn route(
        raw_target: &str,
        configured_proxy: Option<&str>,
        configured_no_proxy: Option<&str>,
        env: &[(&str, &str)],
    ) -> Result<ProxyRoute> {
        let env = env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>();
        select_proxy_route_with_env(
            &Url::parse(raw_target).expect("test URL should parse"),
            configured_proxy,
            configured_no_proxy,
            |name| env.get(name).cloned(),
        )
    }

    #[test]
    fn explicit_proxy_and_no_proxy_select_one_route() {
        assert_eq!(
            route(
                "https://api.example.test/path",
                Some("http://proxy.test:8080"),
                None,
                &[],
            )
            .unwrap(),
            ProxyRoute::from_proxy_url("http://proxy.test:8080").unwrap()
        );
        assert_eq!(
            route(
                "https://api.example.test/path",
                Some("http://proxy.test:8080"),
                Some(".example.test"),
                &[],
            )
            .unwrap(),
            ProxyRoute::Direct
        );
    }

    #[test]
    fn environment_proxy_selection_is_shared_by_http_and_websocket_schemes() {
        assert!(
            route(
                "http://example.test/path",
                None,
                None,
                &[("http_proxy", "http://proxy.test:8080")],
            )
            .unwrap()
            .is_proxy()
        );
        assert!(
            route(
                "ws://example.test/socket",
                None,
                None,
                &[("http_proxy", "http://proxy.test:8080")],
            )
            .unwrap()
            .is_proxy()
        );
        assert!(
            route(
                "wss://example.test/socket",
                None,
                None,
                &[("HTTPS_PROXY", "http://proxy.test:8080")],
            )
            .unwrap()
            .is_proxy()
        );
        assert_eq!(
            route(
                "ws://example.test/socket",
                None,
                None,
                &[("HTTP_PROXY", "http://proxy.test:8080")],
            )
            .unwrap(),
            ProxyRoute::Direct
        );
    }

    #[test]
    fn empty_explicit_proxy_disables_environment_proxy_fallback() {
        assert_eq!(
            route(
                "http://example.test/path",
                Some(""),
                None,
                &[("http_proxy", "http://proxy.test:8080")],
            )
            .unwrap(),
            ProxyRoute::Direct
        );
    }

    #[test]
    fn no_proxy_handles_domains_ip_cidrs_and_trailing_dots() {
        assert!(no_proxy_matches("api.example.test", Some("example.test")));
        assert!(!no_proxy_matches(
            "api.example.test",
            Some("example.test:8443")
        ));
        assert!(!no_proxy_matches("notexample.test", Some("example.test")));
        assert!(no_proxy_matches("anything.test", Some("*")));
        assert!(no_proxy_matches("api.example.test.", Some("example.test.")));
        assert!(no_proxy_matches("192.0.2.42", Some("192.0.2.0/24")));
        assert!(!no_proxy_matches("198.51.100.42", Some("192.0.2.0/24")));
        assert!(no_proxy_matches("[::1]", Some("::1")));
        assert!(!no_proxy_matches("[::1]", Some("[::1]")));
        assert!(!no_proxy_matches("[::1]", Some("[::1]:8443")));
    }

    #[test]
    fn no_proxy_routes_match_libcurl() {
        use std::{
            net::TcpListener,
            time::{Duration, Instant},
        };

        use curl::easy::{Easy, List};

        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        origin.set_nonblocking(true).unwrap();
        proxy.set_nonblocking(true).unwrap();
        let origin_port = origin.local_addr().unwrap().port();
        let proxy_port = proxy.local_addr().unwrap().port();
        let proxy_url = format!("http://127.0.0.1:{proxy_port}");
        let domain_with_port = format!("example.test:{origin_port}");

        for host in [
            "api.example.test",
            "api.example.test.",
            "api.example.test..",
        ] {
            let host_with_port = format!("{host}:{origin_port}");
            for no_proxy in [
                "",
                "api.example.test",
                "example.test",
                ".example.test",
                "example.test.",
                ".example.test.",
                "..example.test",
                "example.test..",
                "..example.test..",
                &host_with_port,
                &domain_with_port,
                "example.test:1",
                "*",
                "*,unrelated.test",
                "unrelated.test,*",
                "unrelated.test,*,.example.test",
                " * ",
            ] {
                // Both candidate endpoints are local listeners. Ask libcurl to
                // connect without sending a request, and observe which port it
                // actually chose rather than duplicating the matcher in the test.
                let mut easy = Easy::new();
                easy.url(&format!("http://{host}:{origin_port}/")).unwrap();
                easy.proxy(&proxy_url).unwrap();
                easy.noproxy(no_proxy).unwrap();
                easy.connect_only(true).unwrap();
                easy.timeout(Duration::from_secs(2)).unwrap();
                let mut resolve = List::new();
                resolve
                    .append(&format!("{host}:{origin_port}:127.0.0.1"))
                    .unwrap();
                easy.resolve(resolve).unwrap();
                easy.perform().unwrap();
                let connected_port = easy.primary_port().unwrap();
                assert!(connected_port == origin_port || connected_port == proxy_port);
                let curl_uses_proxy = connected_port == proxy_port;
                let listener = if curl_uses_proxy { &proxy } else { &origin };
                // Client connect completion does not guarantee the server's
                // nonblocking accept queue is observable in the same turn.
                let deadline = Instant::now() + Duration::from_secs(2);
                let _connection = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("libcurl's selected endpoint did not accept: {error}"),
                    }
                };

                for (scheme, proxy_env) in [
                    ("http", "http_proxy"),
                    ("https", "HTTPS_PROXY"),
                    ("ws", "http_proxy"),
                    ("wss", "HTTPS_PROXY"),
                ] {
                    let target = format!("{scheme}://{host}:{origin_port}/");
                    let configured = route(&target, Some(&proxy_url), Some(no_proxy), &[]).unwrap();
                    let environment = route(
                        &target,
                        None,
                        None,
                        &[(proxy_env, &proxy_url), ("NO_PROXY", no_proxy)],
                    )
                    .unwrap();
                    assert_eq!(
                        configured.is_proxy(),
                        curl_uses_proxy,
                        "configured NO_PROXY={no_proxy:?} for {target}"
                    );
                    assert_eq!(
                        environment.is_proxy(),
                        curl_uses_proxy,
                        "environment NO_PROXY={no_proxy:?} for {target}"
                    );
                }
            }
        }
    }

    #[test]
    fn proxy_parser_normalizes_socks_schemes_to_remote_dns_for_curl() {
        for (raw, scheme, port, curl_url) in [
            (
                "proxy.test:8080",
                ProxyScheme::Http,
                8080,
                "http://proxy.test:8080/",
            ),
            (
                "https://proxy.test",
                ProxyScheme::Https,
                443,
                "https://proxy.test:443/",
            ),
            (
                "socks://proxy.test",
                ProxyScheme::Socks5h,
                1080,
                "socks5h://proxy.test:1080",
            ),
            (
                "socks5://proxy.test",
                ProxyScheme::Socks5h,
                1080,
                "socks5h://proxy.test:1080",
            ),
            (
                "socks5h://proxy.test",
                ProxyScheme::Socks5h,
                1080,
                "socks5h://proxy.test:1080",
            ),
            (
                "socks4://proxy.test",
                ProxyScheme::Socks4a,
                1080,
                "socks4a://proxy.test:1080",
            ),
            (
                "socks4a://proxy.test",
                ProxyScheme::Socks4a,
                1080,
                "socks4a://proxy.test:1080",
            ),
        ] {
            let proxy = SelectedProxy::parse(raw).unwrap();
            assert_eq!(proxy.scheme(), scheme);
            assert_eq!(proxy.endpoint_host(), "proxy.test");
            assert_eq!(proxy.endpoint_port(), port);
            assert_eq!(proxy.target_resolution(), ProxyTargetResolution::Proxy);
            assert_eq!(proxy.curl_url(), curl_url);
            if raw.contains("://") {
                assert_eq!(
                    Url::parse(proxy.url()).unwrap().scheme(),
                    raw.split_once("://").unwrap().0
                );
            }
        }
    }

    #[test]
    fn proxy_parser_keeps_curl_endpoint_port_aligned_with_dns_endpoint() {
        for (raw, expected_port, expected_curl_url) in [
            ("http://proxy.test", 1080, "http://proxy.test:1080/"),
            ("https://proxy.test", 443, "https://proxy.test:443/"),
            ("http://proxy.test:80", 80, "http://proxy.test:80/"),
            ("https://proxy.test:443", 443, "https://proxy.test:443/"),
            ("http://proxy.test:8080", 8080, "http://proxy.test:8080/"),
            ("https://proxy.test:8443", 8443, "https://proxy.test:8443/"),
        ] {
            let proxy = SelectedProxy::parse(raw).unwrap();
            assert_eq!(proxy.endpoint_port(), expected_port, "{raw}");
            assert_eq!(proxy.curl_url(), expected_curl_url, "{raw}");
        }
    }

    #[test]
    fn proxy_dns_pin_matches_libcurl_proxy_ports() {
        use std::{ffi::c_int, time::Duration};

        use curl::easy::{Easy2, Handler};

        #[derive(Default)]
        struct RefuseSockets {
            attempts: usize,
        }

        impl Handler for RefuseSockets {
            fn open_socket(
                &mut self,
                _family: c_int,
                _socktype: c_int,
                _protocol: c_int,
            ) -> Option<curl::multi::Socket> {
                self.attempts += 1;
                None
            }
        }

        for raw in [
            "http://proxy.invalid",
            "https://proxy.invalid",
            "http://proxy.invalid:80",
            "https://proxy.invalid:443",
            "http://proxy.invalid:8080",
            "https://proxy.invalid:8443",
            "socks5h://proxy.invalid",
            "socks4a://proxy.invalid",
        ] {
            let proxy = SelectedProxy::parse(raw).unwrap();
            for curl_proxy in [raw, proxy.curl_url()] {
                let mut easy = Easy2::new(RefuseSockets::default());
                easy.url("http://origin.invalid/proxy-port").unwrap();
                easy.proxy(curl_proxy).unwrap();
                easy.noproxy("").unwrap();
                easy.timeout(Duration::from_secs(1)).unwrap();
                let mut dns = crate::CurlDnsResolution::resolve_endpoint(
                    DnsTarget::new(proxy.endpoint_host(), proxy.endpoint_port()),
                    Vec::new(),
                );
                dns.install(&mut easy, &[IpAddr::from([127, 0, 0, 1])])
                    .unwrap();

                // Let the linked libcurl interpret both URLs. Only Moli's
                // chosen port has a DNS pin for this reserved hostname, so
                // reaching open_socket proves that libcurl selected it too.
                // Refuse the socket before any connection or TLS handshake.
                let error = easy.perform().expect_err("test refuses every socket");
                assert!(error.is_couldnt_connect(), "{curl_proxy}: {error}");
                assert_eq!(easy.get_ref().attempts, 1, "{curl_proxy}");
            }
        }
    }

    #[test]
    fn route_selects_only_the_locally_connected_dns_endpoint() {
        let empty = HostResolveOverrides::default();
        let direct = ProxyRoute::Direct;
        let direct_endpoint = direct
            .connection_dns_endpoint(&Url::parse("https://origin.test/path").unwrap(), &empty)
            .unwrap()
            .unwrap();
        assert_eq!(
            direct_endpoint.role(),
            ConnectionEndpointRole::RequestTarget
        );
        assert_eq!(
            direct_endpoint.target(),
            &DnsTarget::new("origin.test", 443)
        );

        let proxy = ProxyRoute::from_proxy_url("http://proxy.test:8080").unwrap();
        let proxy_endpoint = proxy
            .connection_dns_endpoint(&Url::parse("https://origin.test/path").unwrap(), &empty)
            .unwrap()
            .unwrap();
        assert_eq!(proxy_endpoint.role(), ConnectionEndpointRole::Proxy);
        assert_eq!(proxy_endpoint.target(), &DnsTarget::new("proxy.test", 8080));

        let ip_proxy = ProxyRoute::from_proxy_url("http://192.0.2.1:8080").unwrap();
        assert!(
            ip_proxy
                .connection_dns_endpoint(&Url::parse("https://origin.test/path").unwrap(), &empty,)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn proxy_endpoint_override_avoids_dns_and_target_override_is_rejected() {
        let proxy = ProxyRoute::from_proxy_url("http://proxy.test:8080").unwrap();
        let proxy_override =
            HostResolveOverrides::parse(&["proxy.test:8080:192.0.2.1".to_owned()]).unwrap();
        assert!(
            proxy
                .connection_dns_endpoint(
                    &Url::parse("https://origin.test/path").unwrap(),
                    &proxy_override,
                )
                .unwrap()
                .is_none()
        );

        let target_override =
            HostResolveOverrides::parse(&["origin.test:443:198.51.100.1".to_owned()]).unwrap();
        let error = proxy
            .connection_dns_endpoint(
                &Url::parse("https://origin.test/path").unwrap(),
                &target_override,
            )
            .unwrap_err();
        assert!(error.to_string().contains("would be ignored"), "{error:#}");
    }
}
