use std::{
    future::{Ready, ready},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    task::{Context, Poll},
};

use axum::{
    http::{HeaderMap, Request, StatusCode, header, uri::Authority},
    response::{IntoResponse, Response},
};
use futures_util::future::Either;
use tower::Service;
use url::{Host, Origin, Url};

/// Check every HTTP request, including WebSocket upgrades, before routing.
/// Like Chromium's DevTools server, accept IP literals without resolving DNS.
#[derive(Clone)]
pub(super) struct RequestGuard<S> {
    inner: S,
    server_origin: Origin,
    remote_allow_origins: Arc<[String]>,
}

impl<S> RequestGuard<S> {
    pub(super) fn new(inner: S, addr: SocketAddr, remote_allow_origins: &[String]) -> Self {
        let server_host = match addr.ip() {
            IpAddr::V4(ip) => Host::Ipv4(ip),
            IpAddr::V6(ip) => Host::Ipv6(ip),
        };
        let remote_allow_origins = remote_allow_origins
            .iter()
            .map(|origin| origin.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        Self {
            inner,
            server_origin: Origin::Tuple("http".into(), server_host, addr.port()),
            remote_allow_origins,
        }
    }

    fn allows_origin(&self, headers: &HeaderMap) -> bool {
        let mut origins = headers.get_all(header::ORIGIN).iter();
        let Some(origin) = origins.next() else {
            return true;
        };
        if origins.next().is_some() {
            return false;
        }
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        // Like Chromium's DevTools WebSocket server, compare against the bound
        // IP endpoint rather than trusting the request's Host header.
        Url::parse(origin).is_ok_and(|url| url.origin() == self.server_origin)
            || self
                .remote_allow_origins
                .iter()
                .any(|allowed| allowed == "*" || allowed.eq_ignore_ascii_case(origin))
    }
}

fn allows_host(headers: &HeaderMap) -> bool {
    let mut hosts = headers.get_all(header::HOST).iter();
    let Some(host) = hosts.next() else {
        return false;
    };
    if hosts.next().is_some() {
        return false;
    }
    let Ok(value) = host.to_str() else {
        return false;
    };
    if value.contains('@') {
        return false;
    }
    let Ok(authority) = value.parse::<Authority>() else {
        return false;
    };
    let host = authority.host();
    // Like Chromium, treat an empty port as unspecified. Nonempty ports must
    // be numeric and fit in u16; IPv6 addresses must remain bracketed.
    let Some(suffix) = value.strip_prefix(host) else {
        return false;
    };
    if !suffix.is_empty()
        && !suffix.strip_prefix(':').is_some_and(|port| {
            port.is_empty()
                || (port.bytes().all(|byte| byte.is_ascii_digit()) && port.parse::<u16>().is_ok())
        })
    {
        return false;
    }
    // Match Chromium's net::IsLocalHostname, including one trailing root dot.
    let local_host = host.strip_suffix('.').unwrap_or(host);
    local_host.eq_ignore_ascii_case("localhost")
        || local_host
            .rsplit_once('.')
            .is_some_and(|(_, tld)| tld.eq_ignore_ascii_case("localhost"))
        || host.parse::<Ipv4Addr>().is_ok()
        || host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .is_some_and(|host| host.parse::<Ipv6Addr>().is_ok())
}

impl<S, B> Service<Request<B>> for RequestGuard<S>
where
    S: Service<Request<B>, Response = Response>,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Either<Ready<Result<Response, S::Error>>, S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        let rejection = if !allows_host(request.headers()) {
            Some("Forbidden: Host must be an IP literal or localhost")
        } else if !self.allows_origin(request.headers()) {
            Some(
                "Forbidden: Origin is not allowed; configure --remote-allow-origins for trusted origins",
            )
        } else {
            None
        };
        if let Some(message) = rejection {
            return Either::Left(ready(Ok((StatusCode::FORBIDDEN, message).into_response())));
        }
        Either::Right(self.inner.call(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const SERVER_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9222);

    fn origin_headers(origin: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, origin.parse().unwrap());
        headers
    }

    #[test]
    fn origin_is_optional_and_only_same_origin_is_allowed_by_default() {
        let guard = RequestGuard::new((), SERVER_ADDR, &[]);
        assert!(guard.allows_origin(&HeaderMap::new()));
        for origin in ["http://127.0.0.1:9222", "HTTP://127.0.0.1:09222"] {
            assert!(guard.allows_origin(&origin_headers(origin)), "{origin}");
        }
        for origin in [
            "https://attacker.example",
            "https://127.0.0.1:9222",
            "http://127.0.0.1:8000",
            "http://127.0.0.2:9222",
            "http://localhost:9222",
            "http://localhost:8000",
            "http://app.localhost:9222",
            "null",
            "",
            "not-an-origin",
        ] {
            assert!(!guard.allows_origin(&origin_headers(origin)), "{origin:?}");
        }
    }

    #[test]
    fn same_origin_uses_the_bound_ip_and_effective_port() {
        for (addr, same_origin, other_origin) in [
            ("127.0.0.1:80", "http://127.0.0.1", "http://127.0.0.1:81"),
            (
                "[::1]:9222",
                "http://[0:0:0:0:0:0:0:1]:9222",
                "http://[::1]:8000",
            ),
            (
                "192.168.1.20:9222",
                "http://192.168.1.20:9222",
                "http://127.0.0.1:9222",
            ),
            (
                "0.0.0.0:9222",
                "http://0.0.0.0:9222",
                "http://127.0.0.1:9222",
            ),
            ("[::]:9222", "http://[::]:9222", "http://[::1]:9222"),
        ] {
            let guard = RequestGuard::new((), addr.parse().unwrap(), &[]);
            assert!(guard.allows_origin(&origin_headers(same_origin)), "{addr}");
            assert!(
                guard.allows_origin(&origin_headers(&format!("http://{addr}"))),
                "{addr}"
            );
            assert!(
                !guard.allows_origin(&origin_headers(other_origin)),
                "{addr}"
            );
            assert!(
                !guard.allows_origin(&origin_headers(&format!("https://{addr}"))),
                "{addr}"
            );
        }
    }

    #[test]
    fn origin_allowlist_matches_whole_origins_case_insensitively() {
        let guard = RequestGuard::new(
            (),
            SERVER_ADDR,
            &[
                " HTTPS://Trusted.Example ".into(),
                " http://localhost:3000 ".into(),
                " ".into(),
                "https://other.example".into(),
            ],
        );
        assert!(guard.allows_origin(&HeaderMap::new()));
        for origin in [
            "http://127.0.0.1:9222",
            "https://trusted.example",
            "HTTPS://TRUSTED.EXAMPLE",
            "http://localhost:3000",
            "https://other.example",
        ] {
            assert!(guard.allows_origin(&origin_headers(origin)), "{origin}");
        }
        for origin in [
            "http://trusted.example",
            "https://trusted.example:443",
            "https://trusted.example.attacker.example",
            "https://trusted.example/",
            "http://localhost:3001",
            "https://attacker.example, https://trusted.example",
            "null",
            "",
        ] {
            assert!(!guard.allows_origin(&origin_headers(origin)), "{origin:?}");
        }
    }

    #[test]
    fn wildcard_allows_all_origins_and_null_requires_an_explicit_opt_in() {
        let wildcard = RequestGuard::new((), SERVER_ADDR, &[" * ".into()]);
        for origin in ["https://any.example", "http://localhost:3000", "null", ""] {
            assert!(
                wildcard.allows_origin(&origin_headers(origin)),
                "{origin:?}"
            );
        }
        let opaque = RequestGuard::new((), SERVER_ADDR, &["null".into()]);
        assert!(opaque.allows_origin(&origin_headers("null")));
        assert!(!opaque.allows_origin(&origin_headers("https://any.example")));
    }

    #[test]
    fn duplicate_and_non_text_origin_headers_are_rejected() {
        let guard = RequestGuard::new((), SERVER_ADDR, &["*".into()]);
        let mut headers = origin_headers("http://127.0.0.1:9222");
        headers.append(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(!guard.allows_origin(&headers));
        headers.insert(header::ORIGIN, HeaderValue::from_bytes(b"\xff").unwrap());
        assert!(!guard.allows_origin(&headers));
    }

    #[test]
    fn host_accepts_localhost_and_any_ipv4_or_bracketed_ipv6_literal() {
        for host in [
            "localhost",
            "LOCALHOST:9222",
            "localhost.",
            "LOCALHOST.:9222",
            "app.localhost",
            "APP.LocalHost:9222",
            "app.localhost.",
            "app.localhost.:9222",
            "app.dev.localhost",
            "app.dev.localhost.:9222",
            "localhost:",
            "app.localhost.:",
            "127.0.0.1",
            "127.0.0.1:",
            "127.0.0.1:9222",
            "192.168.1.20:9222",
            "10.0.0.2",
            "203.0.113.7:65535",
            "0.0.0.0:0",
            "[::1]",
            "[::1]:",
            "[::1]:9222",
            "[::]",
            "[2001:db8::1]:9222",
            "[::ffff:192.168.1.20]:9222",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, host.parse().unwrap());
            assert!(allows_host(&headers), "{host}");
        }
    }

    #[test]
    fn host_rejects_domains_userinfo_and_malformed_authorities() {
        for host in [
            "attacker.example",
            "attacker.example:9222",
            "attacker.example:",
            "notlocalhost",
            "localhost.attacker.example",
            "app.localhost.attacker.example",
            "localhost..",
            "app.localhost..",
            "127.0.0.1.attacker.example:9222",
            "localhost@attacker.example",
            "attacker.example@localhost",
            "user@127.0.0.1:9222",
            "user@",
            "@",
            ":9222",
            "http://localhost:9222",
            "localhost/path",
            "localhost?query",
            "localhost#fragment",
            " localhost",
            "localhost ",
            "localhost:bad",
            "localhost:65536",
            "app.localhost.:bad",
            "app.localhost.:65536",
            "localhost:-1",
            "localhost:80:90",
            "::1",
            "::1:9222",
            "[localhost]",
            "[::1]:bad",
            "[::1]:65536",
            "[::1]suffix",
            "127.1",
            "2130706433",
            "",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, host.parse().unwrap());
            assert!(!allows_host(&headers), "{host:?}");
        }
    }

    #[test]
    fn missing_duplicate_and_non_text_host_headers_are_rejected() {
        let mut headers = HeaderMap::new();
        assert!(!allows_host(&headers));
        headers.append(header::HOST, "localhost".parse().unwrap());
        headers.append(header::HOST, "attacker.example".parse().unwrap());
        assert!(!allows_host(&headers));
        headers.insert(header::HOST, HeaderValue::from_bytes(b"\xff").unwrap());
        assert!(!allows_host(&headers));
    }
}
