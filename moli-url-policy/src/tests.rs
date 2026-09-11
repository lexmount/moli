use url::Url;

use super::*;

fn url(value: &str) -> Url {
    Url::parse(value).expect("test URL")
}

#[test]
fn browser_scheme_properties_match_chromium_registries() {
    assert!(BrowserUrlScheme::Http.is_cors_enabled());
    assert!(BrowserUrlScheme::Https.is_cors_enabled());
    assert!(BrowserUrlScheme::Data.is_cors_enabled());
    assert!(!BrowserUrlScheme::File.is_cors_enabled());
    assert!(!BrowserUrlScheme::Blob.is_cors_enabled());

    assert!(BrowserUrlScheme::Http.supports_fetch_api());
    assert!(BrowserUrlScheme::Https.supports_fetch_api());
    assert!(!BrowserUrlScheme::Data.supports_fetch_api());
    assert!(!BrowserUrlScheme::File.supports_fetch_api());

    assert!(BrowserUrlScheme::File.is_local());
    assert!(!BrowserUrlScheme::Data.is_local());
}

#[test]
fn fetch_and_xhr_route_only_http_and_supported_local_resources() {
    let cases = [
        ("http://example.test/", Ok(BrowserUrlRoute::HttpNetwork)),
        ("https://example.test/", Ok(BrowserUrlRoute::HttpNetwork)),
        ("data:text/plain,hello", Ok(BrowserUrlRoute::LocalData)),
        (
            "blob:https://example.test/id",
            Ok(BrowserUrlRoute::LocalBlob),
        ),
    ];
    for (value, expected) in cases {
        assert_eq!(route_fetch_url(&url(value)), expected);
        assert_eq!(route_xml_http_request_url(&url(value)), expected);
    }

    for value in [
        "file:///etc/hostname",
        "about:blank",
        "ws://example.test/socket",
        "ftp://example.test/file",
        "custom:payload",
    ] {
        let fetch_error = route_fetch_url(&url(value)).expect_err("fetch must reject scheme");
        assert_eq!(fetch_error.reason(), UrlPolicyReason::UnsupportedScheme);
        assert_eq!(
            fetch_error.to_string(),
            format!("URL scheme \"{}\" is not supported.", url(value).scheme())
        );
        assert!(route_xml_http_request_url(&url(value)).is_err());
    }
}

#[test]
fn navigation_requires_an_explicit_local_file_capability() {
    let file_url = url("file:///etc/hostname");
    let error = route_navigation_url(&file_url, LocalFileNavigationAccess::Denied)
        .expect_err("hosted navigation must reject file URL");
    assert_eq!(error.reason(), UrlPolicyReason::LocalFileCapabilityRequired);
    assert_eq!(
        route_navigation_url(&file_url, LocalFileNavigationAccess::BrowserGranted),
        Ok(BrowserUrlRoute::LocalFile)
    );

    for value in [
        "about:blank",
        "about:BLANK",
        "about:Blank#fragment",
        "ABOUT:bLaNk?query#fragment",
    ] {
        assert_eq!(
            route_navigation_url(&url(value), LocalFileNavigationAccess::Denied),
            Ok(BrowserUrlRoute::EmptyDocument),
            "{value}"
        );
    }
    assert_eq!(
        route_navigation_url(&url("about:srcdoc"), LocalFileNavigationAccess::Denied)
            .expect_err("about:srcdoc is not a public navigation route")
            .reason(),
        UrlPolicyReason::UnsupportedAboutUrl
    );
}

#[test]
fn file_hosts_drive_letters_and_empty_segments_do_not_change_access_policy() {
    for value in [
        "file://server/C:/private/file",
        "file://[::1]/C|/private/file",
        "file:////private//file",
        "file://localhost////private/file",
    ] {
        let file_url = url(value);
        assert_eq!(
            route_navigation_url(&file_url, LocalFileNavigationAccess::Denied)
                .expect_err("file URL parsing must not grant local access")
                .reason(),
            UrlPolicyReason::LocalFileCapabilityRequired,
        );
        assert_eq!(
            route_navigation_url(&file_url, LocalFileNavigationAccess::BrowserGranted),
            Ok(BrowserUrlRoute::LocalFile),
        );
        assert!(route_fetch_url(&file_url).is_err());
        assert!(route_xml_http_request_url(&file_url).is_err());
        assert!(ensure_http_network_transport_url(&file_url).is_err());
    }
}

#[test]
fn http_transport_never_accepts_a_local_or_non_http_scheme() {
    assert!(ensure_http_network_transport_url(&url("http://example.test/")).is_ok());
    assert!(ensure_http_network_transport_url(&url("https://example.test/")).is_ok());

    for value in [
        "file:///etc/hostname",
        "data:text/plain,hello",
        "blob:https://example.test/id",
        "ws://example.test/socket",
        "ftp://example.test/file",
    ] {
        let error = ensure_http_network_transport_url(&url(value))
            .expect_err("HTTP transport must reject scheme");
        assert_eq!(error.reason(), UrlPolicyReason::NonHttpNetworkScheme);
        assert_eq!(error.context(), UrlRequestContext::HttpNetworkTransport);
    }
}

#[test]
fn service_worker_and_websocket_routes_are_context_specific() {
    assert_eq!(
        route_service_worker_url(&url("https://example.test/sw.js")),
        Ok(BrowserUrlRoute::HttpNetwork)
    );
    assert!(route_service_worker_url(&url("file:///tmp/sw.js")).is_err());
    assert_eq!(
        route_websocket_url(&url("wss://example.test/socket")),
        Ok(BrowserUrlRoute::WebSocket)
    );
    assert!(route_websocket_url(&url("https://example.test/socket")).is_err());
}
