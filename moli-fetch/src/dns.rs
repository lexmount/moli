use anyhow::Result;
use moli_curl::{ConnectionEndpointRole, CurlDnsResolution, HostResolveOverrides};
use moli_dns_resolver::DnsTarget;
use url::Url;

use crate::{FetchConfig, proxy::HttpProxyRoute};

/// Fetch-side DNS admission decision.
///
/// Each request resolves at most the one hostname curl will connect to locally:
/// the origin for a direct route or the proxy endpoint for a proxy route.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FetchCurlDnsAdmission {
    NoSharedResolution,
    DirectTarget(DnsTarget),
    ProxyEndpoint(DnsTarget),
}

pub(crate) fn curl_dns_resolution(
    config: &FetchConfig,
    url: &Url,
    proxy_route: &HttpProxyRoute,
) -> Result<CurlDnsResolution> {
    let host_resolve = HostResolveOverrides::parse(config.http_host_resolve())?;
    let static_entries = host_resolve.normalized_entries();
    match curl_dns_admission(url, proxy_route, &host_resolve)? {
        FetchCurlDnsAdmission::NoSharedResolution => Ok(CurlDnsResolution::no_shared_resolution()),
        FetchCurlDnsAdmission::DirectTarget(target) => {
            let policy = config.network_address_policy();
            Ok(CurlDnsResolution::resolve_endpoint(target, static_entries)
                .with_network_address_policy(policy, url.to_string()))
        }
        FetchCurlDnsAdmission::ProxyEndpoint(target) => {
            // Pin the locally connected proxy, but do not apply origin address
            // policy to it. Configured enterprise proxies commonly live on
            // private networks, and the proxy owns target DNS/admission.
            Ok(CurlDnsResolution::resolve_endpoint(target, static_entries))
        }
    }
}

fn curl_dns_admission(
    url: &Url,
    proxy_route: &HttpProxyRoute,
    host_resolve: &HostResolveOverrides,
) -> Result<FetchCurlDnsAdmission> {
    if !matches!(url.scheme(), "http" | "https") {
        return Ok(FetchCurlDnsAdmission::NoSharedResolution);
    }
    Ok(
        match proxy_route.connection_dns_endpoint(url, host_resolve)? {
            None => FetchCurlDnsAdmission::NoSharedResolution,
            Some(endpoint) => match endpoint.role() {
                ConnectionEndpointRole::RequestTarget => {
                    FetchCurlDnsAdmission::DirectTarget(endpoint.target().clone())
                }
                ConnectionEndpointRole::Proxy => {
                    FetchCurlDnsAdmission::ProxyEndpoint(endpoint.target().clone())
                }
            },
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admission(
        config: &FetchConfig,
        raw_url: &str,
        proxy_route: &HttpProxyRoute,
    ) -> FetchCurlDnsAdmission {
        admission_result(config, raw_url, proxy_route).expect("test DNS admission should succeed")
    }

    fn admission_result(
        config: &FetchConfig,
        raw_url: &str,
        proxy_route: &HttpProxyRoute,
    ) -> Result<FetchCurlDnsAdmission> {
        let host_resolve = HostResolveOverrides::parse(config.http_host_resolve())?;
        curl_dns_admission(
            &Url::parse(raw_url).expect("test URL should parse"),
            proxy_route,
            &host_resolve,
        )
    }

    fn direct_target(host: &str, port: u16) -> FetchCurlDnsAdmission {
        FetchCurlDnsAdmission::DirectTarget(DnsTarget::new(host, port))
    }

    fn proxy_endpoint(host: &str, port: u16) -> FetchCurlDnsAdmission {
        FetchCurlDnsAdmission::ProxyEndpoint(DnsTarget::new(host, port))
    }

    fn proxy_route(raw: &str) -> HttpProxyRoute {
        HttpProxyRoute::from_proxy_url(raw).expect("test proxy URL should parse")
    }

    #[test]
    fn direct_http_and_https_domains_use_shared_resolution() {
        let config = FetchConfig::default();

        assert_eq!(
            admission(&config, "http://example.test/path", &HttpProxyRoute::Direct),
            direct_target("example.test", 80)
        );
        assert_eq!(
            admission(
                &config,
                "https://example.test:8443/path",
                &HttpProxyRoute::Direct,
            ),
            direct_target("example.test", 8443)
        );
    }

    #[test]
    fn ip_literals_and_matching_host_resolve_entries_need_no_shared_resolution() {
        let mut config = FetchConfig::default();

        assert_eq!(
            admission(&config, "http://127.0.0.1/path", &HttpProxyRoute::Direct,),
            FetchCurlDnsAdmission::NoSharedResolution
        );
        assert_eq!(
            admission(&config, "http://[::1]/path", &HttpProxyRoute::Direct,),
            FetchCurlDnsAdmission::NoSharedResolution
        );
        config.set_http_host_resolve(vec!["example.test:80:127.0.0.1".to_owned()]);
        assert_eq!(
            admission(&config, "http://example.test/path", &HttpProxyRoute::Direct,),
            FetchCurlDnsAdmission::NoSharedResolution
        );
        assert_eq!(
            admission(&config, "http://other.test/path", &HttpProxyRoute::Direct,),
            direct_target("other.test", 80),
            "an unrelated host-resolve entry must not return DNS ownership to curl"
        );
    }

    #[test]
    fn selected_proxy_domain_uses_shared_endpoint_resolution() {
        let config = FetchConfig::default();

        for (proxy, port) in [
            ("http://proxy.test:8080", 8080),
            ("https://proxy.test:8443", 8443),
        ] {
            assert_eq!(
                admission(
                    &config,
                    "https://api.example.test/path",
                    &proxy_route(proxy),
                ),
                proxy_endpoint("proxy.test", port)
            );
        }
    }

    #[test]
    fn selected_proxy_ip_needs_no_shared_resolution_even_with_address_policy() {
        let mut config = FetchConfig::default();
        config.set_network_blocking(true, Vec::new());

        assert_eq!(
            admission(
                &config,
                "https://api.example.test/path",
                &proxy_route("http://192.0.2.1:8080"),
            ),
            FetchCurlDnsAdmission::NoSharedResolution
        );
    }

    #[test]
    fn proxy_endpoint_override_needs_no_dns_but_target_override_is_rejected() {
        let route = proxy_route("http://proxy.test:8080");
        let mut config = FetchConfig::default();
        config.set_http_host_resolve(vec!["proxy.test:8080:192.0.2.1".to_owned()]);
        assert_eq!(
            admission(&config, "https://api.example.test/path", &route),
            FetchCurlDnsAdmission::NoSharedResolution
        );

        config.set_http_host_resolve(vec!["api.example.test:443:198.51.100.1".to_owned()]);
        let error = admission_result(&config, "https://api.example.test/path", &route)
            .expect_err("a target override cannot affect remote proxy DNS");
        assert!(error.to_string().contains("would be ignored"), "{error:#}");
    }

    #[test]
    fn address_policy_does_not_reject_proxy_resolved_target() {
        let mut config = FetchConfig::default();
        config.set_network_blocking(true, Vec::new());

        assert_eq!(
            admission(
                &config,
                "https://api.example.test/path",
                &proxy_route("http://proxy.test:8080"),
            ),
            proxy_endpoint("proxy.test", 8080)
        );
    }
}
