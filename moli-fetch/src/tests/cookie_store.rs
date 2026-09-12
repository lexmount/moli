use moli_browser_profile::DEFAULT_SEC_CH_UA_PLATFORM;
use moli_cookie_jar::test_support::BrowserCookieStore;
use moli_cookie_jar::{NetworkCookieRequestContext, new_shared_browser_cookie_store};
use url::Url;

use crate::{
    BrowserNavigationRequestKind, BrowserRequestMetadata, FetchConfig, Request, RequestAuth,
    RequestAuthScheme, RequestAuthTarget, RequestCredentialsMode, RequestMode, RequestResourceType,
    ScriptFetchRequestMetadata, cookie_header_for_request, outgoing_request_headers,
};

#[test]
fn host_only_cookie_uses_default_path_and_path_boundary_matching() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/scoped/index.html").unwrap();
    jar.store_response_headers(
        &response_url,
        &[("set-cookie".to_owned(), "scope=1; HttpOnly".to_owned())],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/scoped/child").unwrap()),
        Some("scope=1".to_owned())
    );
    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/scoped").unwrap()),
        Some("scope=1".to_owned())
    );
    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/scoped-extra").unwrap()),
        None
    );
}

#[test]
fn cookie_replacement_uses_name_domain_and_path_key() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/account/profile").unwrap();

    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "theme=light; Path=/account".to_owned(),
        )],
    );
    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "theme=dark; Path=/account".to_owned(),
        )],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/account/settings").unwrap()),
        Some("theme=dark".to_owned())
    );
}

#[test]
fn invalid_domain_cookie_is_ignored() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/login").unwrap();

    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "session=bad; Domain=other.com; Path=/".to_owned(),
        )],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/").unwrap()),
        None
    );
}

#[test]
fn secure_cookie_only_matches_https_requests() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("https://example.com/login").unwrap();

    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "session=secure; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/").unwrap()),
        None
    );
    assert_eq!(
        jar.cookie_header(&Url::parse("https://example.com/").unwrap()),
        Some("session=secure".to_owned())
    );
}

#[test]
fn max_age_zero_removes_existing_cookie_immediately() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/settings").unwrap();

    jar.store_response_headers(
        &response_url,
        &[("set-cookie".to_owned(), "flash=1; Path=/".to_owned())],
    );
    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "flash=gone; Path=/; Max-Age=0".to_owned(),
        )],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/").unwrap()),
        None
    );
}

#[test]
fn host_only_cookie_does_not_match_subdomains() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/").unwrap();

    jar.store_response_headers(
        &response_url,
        &[("set-cookie".to_owned(), "hostonly=1; Path=/".to_owned())],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://sub.example.com/").unwrap()),
        None
    );
}

#[test]
fn domain_cookie_matches_subdomains_and_normalizes_leading_dot() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/").unwrap();

    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "shared=1; Domain=.example.com; Path=/".to_owned(),
        )],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://sub.example.com/").unwrap()),
        Some("shared=1".to_owned())
    );
}

#[test]
fn invalid_cookie_path_falls_back_to_default_request_path() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/nested/page.html").unwrap();

    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "fallback=1; Path=relative".to_owned(),
        )],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/nested/child").unwrap()),
        Some("fallback=1".to_owned())
    );
    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/relative").unwrap()),
        None
    );
}

#[test]
fn expires_in_past_removes_cookie_immediately() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/session").unwrap();

    jar.store_response_headers(
        &response_url,
        &[("set-cookie".to_owned(), "session=1; Path=/".to_owned())],
    );
    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "session=gone; Path=/; Expires=Wed, 21 Oct 2015 07:28:00 GMT".to_owned(),
        )],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/").unwrap()),
        None
    );
}

#[test]
fn cookies_with_longer_paths_are_sent_first_and_can_coexist() {
    let mut jar = BrowserCookieStore::default();
    let response_url = Url::parse("http://example.com/app/admin/index.html").unwrap();

    jar.store_response_headers(
        &response_url,
        &[("set-cookie".to_owned(), "mode=base; Path=/app".to_owned())],
    );
    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "mode=deep; Path=/app/admin".to_owned(),
        )],
    );

    assert_eq!(
        jar.cookie_header(&Url::parse("http://example.com/app/admin/panel").unwrap()),
        Some("mode=deep; mode=base".to_owned())
    );
}

#[test]
fn request_cookie_header_reads_from_canonical_cookie_core() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://example.com/app/index.html").unwrap();
    let request_url = Url::parse("https://example.com/app/panel").unwrap();

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[(
                "set-cookie".to_owned(),
                "sid=server; Path=/app; Secure".to_owned(),
            )],
        );
    }

    assert_eq!(
        cookie_header_for_request(
            &cookie_store,
            &request_url,
            NetworkCookieRequestContext::subresource("GET"),
        )
        .unwrap(),
        Some("sid=server".to_owned())
    );
}

#[test]
fn outgoing_request_headers_skip_manual_cookie_when_store_cookie_exists() {
    let config = FetchConfig::default();
    let request = Request::new(
        "GET",
        "https://example.com/app/panel",
        None,
        vec![
            ("Cookie".to_owned(), "manual=1".to_owned()),
            ("X-Test".to_owned(), "ok".to_owned()),
        ],
    )
    .unwrap();

    let headers = outgoing_request_headers(&config, &request, Some("sid=server"));

    assert_eq!(
        headers,
        vec![
            ("Cookie".to_owned(), "sid=server".to_owned()),
            ("X-Test".to_owned(), "ok".to_owned()),
            ("Origin".to_owned(), "null".to_owned()),
        ]
    );
}

#[test]
fn outgoing_request_headers_include_default_config_headers() {
    let mut config = FetchConfig::default();
    config.push_default_request_header("X-Test", "one");
    config.push_default_request_header("X-Trace", "two");
    let request = Request::new(
        "GET",
        "https://example.com/app/panel",
        None,
        vec![("X-Request".to_owned(), "three".to_owned())],
    )
    .unwrap();

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        headers,
        vec![
            ("X-Test".to_owned(), "one".to_owned()),
            ("X-Trace".to_owned(), "two".to_owned()),
            ("X-Request".to_owned(), "three".to_owned()),
            ("Origin".to_owned(), "null".to_owned()),
        ]
    );
}

#[test]
fn outgoing_request_headers_skip_default_cookie_when_store_cookie_exists() {
    let mut config = FetchConfig::default();
    config.push_default_request_header("Cookie", "manual=1");
    config.push_default_request_header("X-Test", "ok");
    let request = Request::new("GET", "https://example.com/app/panel", None, vec![]).unwrap();

    let headers = outgoing_request_headers(&config, &request, Some("sid=server"));

    assert_eq!(
        headers,
        vec![
            ("Cookie".to_owned(), "sid=server".to_owned()),
            ("X-Test".to_owned(), "ok".to_owned()),
            ("Origin".to_owned(), "null".to_owned()),
        ]
    );
}

#[test]
fn outgoing_request_headers_keep_default_cookie_without_store_cookie() {
    let mut config = FetchConfig::default();
    config.push_default_request_header("Cookie", "manual=1");
    config.push_default_request_header("X-Test", "ok");
    let request = Request::new("GET", "https://example.com/app/panel", None, vec![]).unwrap();

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        headers,
        vec![
            ("Cookie".to_owned(), "manual=1".to_owned()),
            ("X-Test".to_owned(), "ok".to_owned()),
            ("Origin".to_owned(), "null".to_owned()),
        ]
    );
}

#[test]
fn outgoing_request_headers_send_basic_authorization_preemptively_for_auth_request() {
    let config = FetchConfig::default();
    let request = Request::get("https://example.com/secure")
        .unwrap()
        .with_auth(RequestAuth {
            target: RequestAuthTarget::Server,
            scheme: RequestAuthScheme::Basic,
            username: "aladdin".to_owned(),
            password: "opensesame".to_owned(),
        });

    let headers = outgoing_request_headers(&config, &request, None);
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "Authorization"
                && value == "Basic YWxhZGRpbjpvcGVuc2VzYW1l"),
        "Basic auth continuation must be preemptive so streaming response-stage interception does not pause on the auth challenge response"
    );
}

#[test]
fn outgoing_request_headers_preserve_duplicate_default_headers_in_order() {
    let mut config = FetchConfig::default();
    config.push_default_request_header("X-Test", "one");
    config.push_default_request_header("X-Test", "two");
    let request = Request::new(
        "GET",
        "https://example.com/app/panel",
        None,
        vec![("X-Test".to_owned(), "three".to_owned())],
    )
    .unwrap();

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        headers,
        vec![
            ("X-Test".to_owned(), "one".to_owned()),
            ("X-Test".to_owned(), "two".to_owned()),
            ("X-Test".to_owned(), "three".to_owned()),
            ("Origin".to_owned(), "null".to_owned()),
        ]
    );
}

fn top_level_navigation_request(request_url: &str, initiator_url: Option<&str>) -> Request {
    let request = Request::get(request_url).unwrap();
    if let Some(initiator_url) = initiator_url {
        request.with_initiator_url(&Url::parse(initiator_url).unwrap())
    } else {
        request
    }
}

#[test]
fn top_level_navigation_headers_default_to_browser_style_document_navigation() {
    let config = FetchConfig::default();
    let request = top_level_navigation_request("https://example.com/docs", None);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "accept"),
        Some(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"
        )
    );
    assert_eq!(
        header_value(&headers, "accept-language"),
        Some("en-US,en;q=0.9")
    );
    assert_eq!(
        header_value(&headers, "upgrade-insecure-requests"),
        Some("1")
    );
    assert_eq!(header_value(&headers, "sec-fetch-site"), Some("none"));
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("navigate"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("document"));
    assert_eq!(header_value(&headers, "sec-fetch-user"), Some("?1"));
    assert_eq!(header_value(&headers, "sec-ch-ua-mobile"), Some("?0"));
    assert_eq!(
        header_value(&headers, "sec-ch-ua-platform"),
        Some(DEFAULT_SEC_CH_UA_PLATFORM)
    );
    assert_eq!(
        header_value(&headers, "sec-ch-ua"),
        Some("\"Not:A-Brand\";v=\"99\", \"Google Chrome\";v=\"145\", \"Chromium\";v=\"145\"")
    );
    assert_eq!(header_value(&headers, "referer"), None);
    assert_eq!(header_value(&headers, "cache-control"), None);
}

#[test]
fn generic_request_promoted_to_navigation_uses_navigate_mode_without_origin() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://example.com/docs", None, Vec::new())
        .unwrap()
        .with_top_level_navigation_cookie_context();

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(request.request_mode, crate::RequestMode::Navigate);
    assert_eq!(header_value(&headers, "origin"), None);
}

#[test]
fn headless_navigation_client_hints_follow_chromium_brand_grease_order() {
    let mut config = FetchConfig::default();
    config.set_user_agent(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/145.0.0.0 Safari/537.36",
    );
    let request = top_level_navigation_request("https://example.com/docs", None);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "sec-ch-ua"),
        Some("\"Not:A-Brand\";v=\"99\", \"HeadlessChrome\";v=\"145\", \"Chromium\";v=\"145\"")
    );
}

#[test]
fn overridden_navigation_identity_drives_language_and_low_entropy_headers_together() {
    let mut config = FetchConfig::default();
    config.set_user_agent(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36",
    );
    config.push_default_request_header("Accept-Language", "fr-CA,fr;q=0.8,en;q=0.5");
    let request = top_level_navigation_request("https://example.com/docs", None);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "sec-ch-ua"),
        Some("\"Chromium\";v=\"146\", \"Not-A.Brand\";v=\"24\", \"Google Chrome\";v=\"146\"")
    );
    assert_eq!(
        header_value(&headers, "accept-language"),
        Some("fr-CA,fr;q=0.8,en;q=0.5")
    );
    assert_eq!(config.browser_identity().languages(), ["fr-CA", "fr", "en"]);
    assert_eq!(config.browser_identity().full_version(), "146.1.2.3");
}

#[test]
fn ua_only_override_without_chromium_metadata_omits_client_hint_headers() {
    let mut config = FetchConfig::default();
    config.set_user_agent("CustomAgent/1.0");
    let request = top_level_navigation_request("https://example.com/docs", None);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "user-agent"), None);
    assert_eq!(header_value(&headers, "sec-ch-ua"), None);
    assert_eq!(header_value(&headers, "sec-ch-ua-mobile"), None);
    assert_eq!(header_value(&headers, "sec-ch-ua-platform"), None);
}

#[test]
fn same_origin_reload_navigation_headers_look_like_chromium_reload() {
    let config = FetchConfig::default();
    let request =
        top_level_navigation_request("https://example.com/docs", Some("https://example.com/docs"))
            .with_browser_navigation_kind(BrowserNavigationRequestKind::Reload);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "sec-fetch-site"),
        Some("same-origin")
    );
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("navigate"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("document"));
    assert_eq!(header_value(&headers, "sec-fetch-user"), None);
    assert_eq!(
        header_value(&headers, "referer"),
        Some("https://example.com/docs")
    );
    assert_eq!(header_value(&headers, "cache-control"), Some("max-age=0"));
}

#[test]
fn repeated_protocol_navigation_to_same_url_does_not_look_like_reload() {
    let config = FetchConfig::default();
    let request =
        top_level_navigation_request("https://example.com/docs", Some("https://example.com/docs"));

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "sec-fetch-site"),
        Some("same-origin")
    );
    assert_eq!(
        header_value(&headers, "referer"),
        Some("https://example.com/docs")
    );
    assert_eq!(header_value(&headers, "cache-control"), None);
}

#[test]
fn browser_initiated_navigation_keeps_site_context_without_inferred_referer() {
    let config = FetchConfig::default();
    let request =
        top_level_navigation_request("https://example.com/docs", Some("https://example.com/docs"))
            .without_inferred_referrer();

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "sec-fetch-site"),
        Some("same-origin")
    );
    assert_eq!(header_value(&headers, "referer"), None);
    assert_eq!(header_value(&headers, "cache-control"), None);
}

#[test]
fn generic_subresource_requests_do_not_inherit_browser_headers_without_metadata() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://example.com/app.js", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/docs").unwrap());

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), None);
    assert_eq!(header_value(&headers, "accept-language"), None);
    assert_eq!(header_value(&headers, "upgrade-insecure-requests"), None);
    assert_eq!(header_value(&headers, "sec-fetch-mode"), None);
    assert_eq!(header_value(&headers, "sec-fetch-dest"), None);
    assert_eq!(header_value(&headers, "sec-fetch-site"), None);
    assert_eq!(header_value(&headers, "sec-fetch-user"), None);
    assert_eq!(header_value(&headers, "sec-ch-ua"), None);
    assert_eq!(header_value(&headers, "sec-ch-ua-mobile"), None);
    assert_eq!(header_value(&headers, "sec-ch-ua-platform"), None);
}

#[test]
fn browser_fetch_and_xhr_subresource_headers_match_chromium_same_origin_shape() {
    let config = FetchConfig::default();

    for metadata in [BrowserRequestMetadata::Fetch, BrowserRequestMetadata::Xhr] {
        let request = Request::new("GET", "https://example.com/api/data", None, vec![])
            .unwrap()
            .with_initiator_url(
                &Url::parse("https://example.com/docs/page.html?x=1#section").unwrap(),
            )
            .with_browser_request_metadata(metadata);

        let headers = outgoing_request_headers(&config, &request, None);

        assert_eq!(header_value(&headers, "accept"), Some("*/*"));
        assert_eq!(
            header_value(&headers, "accept-language"),
            Some("en-US,en;q=0.9")
        );
        assert_eq!(
            header_value(&headers, "sec-fetch-site"),
            Some("same-origin")
        );
        assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("cors"));
        assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("empty"));
        assert_eq!(header_value(&headers, "sec-fetch-user"), None);
        assert_eq!(header_value(&headers, "origin"), None);
        assert_eq!(
            header_value(&headers, "sec-ch-ua"),
            Some("\"Not:A-Brand\";v=\"99\", \"Google Chrome\";v=\"145\", \"Chromium\";v=\"145\"")
        );
        assert_eq!(header_value(&headers, "sec-ch-ua-mobile"), Some("?0"));
        assert_eq!(
            header_value(&headers, "sec-ch-ua-platform"),
            Some(DEFAULT_SEC_CH_UA_PLATFORM)
        );
        assert_eq!(
            header_value(&headers, "referer"),
            Some("https://example.com/docs/page.html?x=1")
        );
    }
}

#[test]
fn browser_audio_worklet_subresource_headers_use_audioworklet_destination() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://example.com/worklet.js", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/docs/page.html").unwrap())
        .with_browser_request_metadata(BrowserRequestMetadata::AudioWorklet);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), Some("*/*"));
    assert_eq!(
        header_value(&headers, "sec-fetch-site"),
        Some("same-origin")
    );
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("cors"));
    assert_eq!(
        header_value(&headers, "sec-fetch-dest"),
        Some("audioworklet")
    );
    assert_eq!(header_value(&headers, "origin"), None);
}

#[test]
fn browser_media_subresource_headers_preserve_audio_video_destinations() {
    let config = FetchConfig::default();
    for (metadata, destination) in [
        (BrowserRequestMetadata::Audio, "audio"),
        (BrowserRequestMetadata::Video, "video"),
    ] {
        let request = Request::new("GET", "https://cdn.example/media", None, vec![])
            .unwrap()
            .with_initiator_url(&Url::parse("https://example.com/page").unwrap())
            .with_request_mode(RequestMode::NoCors)
            .with_resource_type(RequestResourceType::Media)
            .with_browser_request_metadata(metadata);

        let headers = outgoing_request_headers(&config, &request, None);

        assert_eq!(header_value(&headers, "accept"), Some("*/*"));
        assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("no-cors"));
        assert_eq!(header_value(&headers, "sec-fetch-dest"), Some(destination));
        assert_eq!(header_value(&headers, "sec-fetch-site"), Some("cross-site"));
    }
}

#[test]
fn browser_image_subresource_headers_use_image_accept_and_destination() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://cdn.example/hero.png", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/page").unwrap())
        .with_request_mode(RequestMode::NoCors)
        .with_resource_type(RequestResourceType::Image)
        .with_browser_request_metadata(BrowserRequestMetadata::Image);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "accept"),
        Some("image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8")
    );
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("no-cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("image"));
    assert_eq!(header_value(&headers, "sec-fetch-site"), Some("cross-site"));
}

#[test]
fn browser_font_subresource_headers_use_cors_font_destination() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://cdn.example/demo.woff2", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/page").unwrap())
        .with_request_mode(RequestMode::Cors)
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_resource_type(RequestResourceType::Font)
        .with_browser_request_metadata(BrowserRequestMetadata::Font);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), Some("*/*"));
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("font"));
    assert_eq!(header_value(&headers, "sec-fetch-site"), Some("cross-site"));
    assert_eq!(
        header_value(&headers, "origin"),
        Some("https://example.com")
    );
}

#[test]
fn browser_text_track_headers_use_vtt_accept_and_track_destination() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://cdn.example/captions.vtt", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://cdn.example/page").unwrap())
        .with_request_mode(RequestMode::SameOrigin)
        .with_resource_type(RequestResourceType::TextTrack)
        .with_browser_request_metadata(BrowserRequestMetadata::TextTrack);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), Some("text/vtt,*/*;q=0.1"));
    assert_eq!(
        header_value(&headers, "sec-fetch-mode"),
        Some("same-origin")
    );
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("track"));
    assert_eq!(
        header_value(&headers, "sec-fetch-site"),
        Some("same-origin")
    );
}

#[test]
fn browser_json_module_subresource_headers_use_json_destination() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://example.com/data.json", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/docs/page.html").unwrap())
        .with_browser_request_metadata(BrowserRequestMetadata::JsonModule);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), Some("*/*"));
    assert_eq!(
        header_value(&headers, "sec-fetch-site"),
        Some("same-origin")
    );
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("json"));
    assert_eq!(header_value(&headers, "origin"), None);
}

#[test]
fn browser_manifest_subresource_headers_match_chromium() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://example.com/app.webmanifest", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/docs/page.html").unwrap())
        .with_request_mode(RequestMode::Cors)
        .with_credentials_mode(RequestCredentialsMode::Omit)
        .with_resource_type(RequestResourceType::Manifest)
        .with_browser_request_metadata(BrowserRequestMetadata::Manifest);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), Some("*/*"));
    assert_eq!(
        header_value(&headers, "sec-fetch-site"),
        Some("same-origin")
    );
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("manifest"));
    assert_eq!(header_value(&headers, "origin"), None);
}

#[test]
fn browser_style_module_subresource_headers_use_style_destination() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://example.com/sheet.css", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/docs/page.html").unwrap())
        .with_browser_request_metadata(BrowserRequestMetadata::StyleModule);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), Some("*/*"));
    assert_eq!(
        header_value(&headers, "sec-fetch-site"),
        Some("same-origin")
    );
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("style"));
    assert_eq!(header_value(&headers, "origin"), None);
}

#[test]
fn browser_stylesheet_subresource_headers_follow_request_mode() {
    let config = FetchConfig::default();
    let initiator = Url::parse("https://example.com/docs/page.html").unwrap();
    let plain = Request::new("GET", "https://cdn.example.test/plain.css", None, vec![])
        .unwrap()
        .with_initiator_url(&initiator)
        .with_request_mode(crate::RequestMode::NoCors)
        .with_browser_request_metadata(BrowserRequestMetadata::Style);
    let anonymous = Request::new(
        "GET",
        "https://cdn.example.test/anonymous.css",
        None,
        vec![],
    )
    .unwrap()
    .with_initiator_url(&initiator)
    .with_request_mode(crate::RequestMode::Cors)
    .with_credentials_mode(crate::RequestCredentialsMode::SameOrigin)
    .with_browser_request_metadata(BrowserRequestMetadata::Style);

    let plain_headers = outgoing_request_headers(&config, &plain, None);
    assert_eq!(header_value(&plain_headers, "accept"), Some("*/*"));
    assert_eq!(
        header_value(&plain_headers, "sec-fetch-mode"),
        Some("no-cors")
    );
    assert_eq!(
        header_value(&plain_headers, "sec-fetch-dest"),
        Some("style")
    );
    assert_eq!(header_value(&plain_headers, "origin"), None);

    let anonymous_headers = outgoing_request_headers(&config, &anonymous, None);
    assert_eq!(
        header_value(&anonymous_headers, "sec-fetch-mode"),
        Some("cors")
    );
    assert_eq!(
        header_value(&anonymous_headers, "sec-fetch-dest"),
        Some("style")
    );
    assert_eq!(
        header_value(&anonymous_headers, "origin"),
        Some("https://example.com")
    );
}

#[test]
fn browser_fetch_subresource_headers_use_cross_site_sec_fetch_site_when_needed() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://www.zhihu.com/api/v4/feed", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://sub.example.com/docs/page.html").unwrap())
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "sec-fetch-site"), Some("cross-site"));
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("empty"));
    assert_eq!(
        header_value(&headers, "origin"),
        Some("https://sub.example.com")
    );
    assert_eq!(
        header_value(&headers, "referer"),
        Some("https://sub.example.com/")
    );
}

#[test]
fn browser_fetch_subresource_headers_use_request_mode_for_sec_fetch_mode() {
    let config = FetchConfig::default();
    let request = Request::new("GET", "https://www.zhihu.com/api/v4/feed", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://sub.example.com/docs/page.html").unwrap())
        .with_request_mode(RequestMode::NoCors)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "sec-fetch-site"), Some("cross-site"));
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("no-cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("empty"));
}

#[test]
fn browser_beacon_subresource_headers_use_no_cors_shape() {
    let config = FetchConfig::default();
    let request = Request::new(
        "POST",
        "https://www.zhihu.com/beacon",
        Some(String::new()),
        vec![],
    )
    .unwrap()
    .with_initiator_url(&Url::parse("https://sub.example.com/docs/page.html").unwrap())
    .with_resource_type(crate::RequestResourceType::Beacon)
    .with_request_mode(RequestMode::NoCors)
    .with_browser_request_metadata(BrowserRequestMetadata::Beacon);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), Some("*/*"));
    assert_eq!(header_value(&headers, "sec-fetch-site"), Some("cross-site"));
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("no-cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("empty"));
}

#[test]
fn browser_ping_subresource_headers_use_no_cors_shape() {
    let config = FetchConfig::default();
    let request = Request::new(
        "POST",
        "https://www.zhihu.com/ping",
        Some("PING".to_owned()),
        vec![
            ("Content-Type".to_owned(), "text/ping".to_owned()),
            (
                "Ping-To".to_owned(),
                "https://sub.example.com/page#next".to_owned(),
            ),
        ],
    )
    .unwrap()
    .with_initiator_url(&Url::parse("https://sub.example.com/docs/page.html").unwrap())
    .with_resource_type(crate::RequestResourceType::Ping)
    .with_request_mode(RequestMode::NoCors)
    .with_browser_request_metadata(BrowserRequestMetadata::Ping);

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(header_value(&headers, "accept"), Some("*/*"));
    assert_eq!(header_value(&headers, "content-type"), Some("text/ping"));
    assert_eq!(
        header_value(&headers, "ping-to"),
        Some("https://sub.example.com/page#next")
    );
    assert_eq!(header_value(&headers, "sec-fetch-site"), Some("cross-site"));
    assert_eq!(header_value(&headers, "sec-fetch-mode"), Some("no-cors"));
    assert_eq!(header_value(&headers, "sec-fetch-dest"), Some("empty"));
}

#[test]
fn explicit_navigation_headers_override_browser_defaults() {
    let config = FetchConfig::default();
    let mut request = Request::get("https://example.com/docs")
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/source").unwrap())
        .with_top_level_navigation_cookie_context();
    request.request_headers = vec![
        ("Accept-Language".to_owned(), "zh-CN,zh;q=0.9".to_owned()),
        ("Sec-Fetch-Site".to_owned(), "cross-site".to_owned()),
        ("Sec-Fetch-User".to_owned(), "?0".to_owned()),
        ("Cache-Control".to_owned(), "no-cache".to_owned()),
    ];

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "accept-language"),
        Some("zh-CN,zh;q=0.9")
    );
    assert_eq!(header_value(&headers, "sec-fetch-site"), Some("cross-site"));
    assert_eq!(header_value(&headers, "sec-fetch-user"), Some("?0"));
    assert_eq!(header_value(&headers, "cache-control"), Some("no-cache"));
}

fn script_request_with_referrer_policy(
    request_url: &str,
    initiator_url: &str,
    referrer_policy: Option<&str>,
    request_headers: Vec<(String, String)>,
) -> Request {
    script_request_with_referrer_policies(
        request_url,
        initiator_url,
        referrer_policy,
        None,
        request_headers,
    )
}

fn script_request_with_referrer_policies(
    request_url: &str,
    initiator_url: &str,
    referrer_policy: Option<&str>,
    document_referrer_policy: Option<&str>,
    request_headers: Vec<(String, String)>,
) -> Request {
    Request::new("GET", request_url, None, request_headers)
        .unwrap()
        .with_initiator_url(&Url::parse(initiator_url).unwrap())
        .with_script_fetch_metadata(ScriptFetchRequestMetadata {
            referrer_policy: referrer_policy.map(str::to_owned),
            document_referrer_policy: document_referrer_policy.map(str::to_owned),
            ..ScriptFetchRequestMetadata::default()
        })
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[test]
fn script_referrer_policy_defaults_to_strict_origin_when_cross_origin() {
    let config = FetchConfig::default();
    let same_origin = script_request_with_referrer_policy(
        "https://example.com/app.js",
        "https://example.com/docs/page.html?x=1#section",
        None,
        Vec::new(),
    );
    let cross_origin = script_request_with_referrer_policy(
        "https://cdn.example/app.js",
        "https://example.com/docs/page.html?x=1#section",
        None,
        Vec::new(),
    );
    let downgrade = script_request_with_referrer_policy(
        "http://cdn.example/app.js",
        "https://example.com/docs/page.html?x=1#section",
        None,
        Vec::new(),
    );

    assert_eq!(
        header_value(
            &outgoing_request_headers(&config, &same_origin, None),
            "referer"
        ),
        Some("https://example.com/docs/page.html?x=1")
    );
    assert_eq!(
        header_value(
            &outgoing_request_headers(&config, &cross_origin, None),
            "referer"
        ),
        Some("https://example.com/")
    );
    assert_eq!(
        header_value(
            &outgoing_request_headers(&config, &downgrade, None),
            "referer"
        ),
        None
    );
}

#[test]
fn script_referrer_policy_variants_control_referer_header() {
    let config = FetchConfig::default();
    let cases = [
        ("no-referrer", "https://cdn.example/app.js", None),
        (
            "no-referrer-when-downgrade",
            "http://cdn.example/app.js",
            None,
        ),
        (
            "origin",
            "http://cdn.example/app.js",
            Some("https://example.com/"),
        ),
        (
            "origin-when-cross-origin",
            "https://cdn.example/app.js",
            Some("https://example.com/"),
        ),
        ("same-origin", "https://cdn.example/app.js", None),
        ("strict-origin", "http://cdn.example/app.js", None),
        (
            "unsafe-url",
            "http://cdn.example/app.js",
            Some("https://example.com/docs/page.html?x=1"),
        ),
    ];

    for (policy, request_url, expected) in cases {
        let request = script_request_with_referrer_policy(
            request_url,
            "https://example.com/docs/page.html?x=1#section",
            Some(policy),
            Vec::new(),
        );
        assert_eq!(
            header_value(
                &outgoing_request_headers(&config, &request, None),
                "referer"
            ),
            expected,
            "unexpected referer for policy {policy}"
        );
    }
}

#[test]
fn script_referrer_policy_preserves_manual_referer_header() {
    let config = FetchConfig::default();
    let request = script_request_with_referrer_policy(
        "https://cdn.example/app.js",
        "https://example.com/docs/page.html?x=1#section",
        Some("no-referrer"),
        vec![("Referer".to_owned(), "https://manual.example/".to_owned())],
    );

    let headers = outgoing_request_headers(&config, &request, None);

    assert_eq!(
        header_value(&headers, "referer"),
        Some("https://manual.example/")
    );
    assert_eq!(
        headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("referer"))
            .count(),
        1
    );
}

#[test]
fn script_referrer_policy_falls_back_to_document_policy() {
    let config = FetchConfig::default();
    let document_policy = script_request_with_referrer_policies(
        "https://cdn.example/app.js",
        "https://example.com/docs/page.html?x=1#section",
        None,
        Some("no-referrer"),
        Vec::new(),
    );
    let element_policy = script_request_with_referrer_policies(
        "https://cdn.example/app.js",
        "https://example.com/docs/page.html?x=1#section",
        Some("origin"),
        Some("no-referrer"),
        Vec::new(),
    );

    assert_eq!(
        header_value(
            &outgoing_request_headers(&config, &document_policy, None),
            "referer"
        ),
        None
    );
    assert_eq!(
        header_value(
            &outgoing_request_headers(&config, &element_policy, None),
            "referer"
        ),
        Some("https://example.com/")
    );
}

#[test]
fn script_referrer_policy_uses_origin_when_full_referrer_is_too_long() {
    let config = FetchConfig::default();
    let long_path = format!("https://example.com/docs/{}?x=1#section", "a".repeat(4100));
    let request = script_request_with_referrer_policy(
        "https://example.com/app.js",
        &long_path,
        Some("unsafe-url"),
        Vec::new(),
    );

    assert_eq!(
        header_value(
            &outgoing_request_headers(&config, &request, None),
            "referer"
        ),
        Some("https://example.com/")
    );
}

#[test]
fn request_cookie_header_includes_document_cookie_mutations() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://example.com/app/index.html").unwrap();
    let request_url = Url::parse("https://example.com/app/panel").unwrap();

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[(
                "set-cookie".to_owned(),
                "sid=server; Path=/app; Secure".to_owned(),
            )],
        );
        jar.set_document_cookie(&response_url, "sid=client; Path=/app");
    }

    assert_eq!(
        cookie_header_for_request(
            &cookie_store,
            &request_url,
            NetworkCookieRequestContext::subresource("GET"),
        )
        .unwrap(),
        Some("sid=client".to_owned())
    );
}
