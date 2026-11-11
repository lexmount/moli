use super::request_security::Transport;
use super::*;
use crate::{RedirectInfo, RedirectSource};

fn request_cookies(request: &str) -> Vec<&str> {
    let mut cookies: Vec<_> = request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .flat_map(|(_, value)| value.split(';').map(str::trim))
        .collect();
    cookies.sort_unstable();
    cookies
}

fn redirect(from: &Url, to: &Url, status: u16) -> RedirectInfo {
    RedirectInfo {
        source: RedirectSource::Network,
        from_url: from.clone(),
        to_url: to.clone(),
        status,
        headers: vec![("Location".to_owned(), to.as_str().as_bytes().to_vec())],
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }
}

#[test]
fn caller_followed_redirect_keeps_referrer_separate_from_cookie_origin() -> Result<()> {
    let origin = Url::parse("https://example.test/")?;
    let blank = Url::parse("about:blank")?;
    let start = origin.join("/start")?;
    let middle = origin.join("/middle")?;
    let end = origin.join("/end")?;
    let mut request = Request::new_browser("GET", start.clone(), None, vec![], (&origin).into())
        .with_initiator_url(&blank)
        .with_browser_site_context(
            moli_cookie_jar::BrowserCookieFacadeContext::default()
                .with_site_for_cookies_url(&origin)
                .with_top_frame_origin_url(&origin),
        );
    assert_eq!(request.referrer_header_value(&start), None);
    for (from, to) in [(&start, &middle), (&middle, &end)] {
        let mut redirect = redirect(from, to, 302);
        redirect
            .headers
            .push(("Referrer-Policy".to_owned(), b"unsafe-url".to_vec()));
        request.follow_redirect(redirect)?;
        assert_eq!(request.cookie_context.initiator_url.as_ref(), Some(&origin));
        assert_eq!(request.referrer_header_value(to), None);
        assert_eq!(
            request.network_cookie_context().site_context,
            moli_cookie_jar::test_support::NetworkSiteContext::same_site()
        );
    }
    Ok(())
}

fn seed_redirect_cookies(
    cookies: &moli_cookie_jar::SharedBrowserCookieStore,
    url: &Url,
    path: &str,
) {
    let mut store = cookies.lock().expect("cookie store is available");
    store.store_response_headers(
        url,
        &[
            (
                "Set-Cookie".to_owned(),
                format!("strict=1; SameSite=Strict; Path={path}").into_bytes(),
            ),
            (
                "Set-Cookie".to_owned(),
                format!("lax=1; SameSite=Lax; Path={path}").into_bytes(),
            ),
            (
                "Set-Cookie".to_owned(),
                format!("unspecified=1; Path={path}").into_bytes(),
            ),
        ],
    );
    let mut aged = store
        .cookies()
        .into_iter()
        .find(|cookie| cookie.name == "unspecified")
        .unwrap();
    aged.name = "aged".to_owned();
    aged.creation_time = Some(
        aged.creation_time
            .expect("seeded cookie records creation time")
            - Duration::from_secs(600),
    );
    store.upsert(aged, moli_cookie_jar::CookieSource::Cdp);
    assert_eq!(
        store
            .cookies()
            .into_iter()
            .find(|cookie| cookie.name == "aged")
            .unwrap()
            .effective_same_site(),
        moli_cookie_jar::StoredCookieEffectiveSameSite::Lax,
        "aged fixture must be outside the unsafe allowance window"
    );
}

async fn assert_redirect_cookie_method_updates(transport: Transport) -> Result<()> {
    for status in [301, 302, 303, 307, 308] {
        for (cross_site_initiator, navigation, force_cross_site) in [
            (false, true, false),
            (true, true, false),
            (false, true, true),
            (false, false, false),
        ] {
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("target")]);
            let mut target_url = Url::parse(&target.url())?;
            target_url.set_host(Some("other.test"))?;
            let mut response = ScriptedResponse::status(status, "Redirect")
                .with_header("Location", target_url.as_str());
            if cross_site_initiator {
                response = response
                    .with_header("Set-Cookie", "redirect_lax=1; SameSite=Lax; Path=/")
                    .with_header("Set-Cookie", "redirect_default=1; Path=/");
            }
            let source = ScriptedHttpServer::spawn(vec![response]);
            let mut source_url = Url::parse(&source.url())?;
            if cross_site_initiator {
                source_url.set_host(Some("other.test"))?;
            }
            let initiator = if cross_site_initiator {
                Url::parse("http://initiator.test/page")?
            } else {
                source_url.clone()
            };
            let mut config = FetchConfig::default();
            config.set_http_no_proxy(Some("*".to_owned()));
            config.set_http_host_resolve(vec![
                format!("other.test:{}:127.0.0.1", target_url.port().unwrap()),
                format!("other.test:{}:127.0.0.1", source_url.port().unwrap()),
            ]);
            let cookies = new_shared_browser_cookie_store();
            seed_redirect_cookies(&cookies, &target_url, "/");
            let client = FetchClient::new(&config, cookies);
            let mut request = Request::new(
                "POST",
                source_url.as_str(),
                Some("payload".to_owned()),
                vec![],
            )?;
            if navigation {
                request = request.with_top_level_navigation_cookie_context();
            }
            request = request.with_initiator_url(&initiator);
            if force_cross_site {
                request = request.with_cross_site_cookie_context();
            }
            transport.fetch(&client, request).await?;
            let source_requests = source.requests();
            let target_requests = target.requests();
            source.shutdown();
            target.shutdown();
            assert_eq!(source_requests.len(), 1);
            assert!(source_requests[0].starts_with("POST "));
            assert_eq!(target_requests.len(), 1);
            let rewritten = matches!(status, 301..=303);
            let expected_method = if rewritten { "GET" } else { "POST" };
            assert!(target_requests[0].starts_with(&format!("{expected_method} ")));
            let mut expected = Vec::new();
            if navigation && !force_cross_site {
                expected.push("unspecified=1");
                if rewritten {
                    expected.extend(["aged=1", "lax=1"]);
                }
                if cross_site_initiator {
                    expected.push("redirect_default=1");
                    if rewritten {
                        expected.push("redirect_lax=1");
                    }
                }
            }
            expected.sort_unstable();
            assert_eq!(
                request_cookies(&target_requests[0]),
                expected,
                "{transport:?}, status={status}, cross_site_initiator={cross_site_initiator}, navigation={navigation}, force_cross_site={force_cross_site}: {}",
                target_requests[0]
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn buffered_redirect_cookies_use_the_current_method() -> Result<()> {
    assert_redirect_cookie_method_updates(Transport::Buffered).await
}

#[tokio::test]
async fn html_redirect_cookies_use_the_current_method() -> Result<()> {
    assert_redirect_cookie_method_updates(Transport::Html).await
}

#[tokio::test]
async fn raw_redirect_cookies_use_the_current_method() -> Result<()> {
    assert_redirect_cookie_method_updates(Transport::Raw).await
}

#[test]
fn caller_followed_redirect_updates_method_without_restoring_strict_context() -> Result<()> {
    use moli_cookie_jar::test_support::{
        NetworkSameSiteContext::{SameSiteLax, SameSiteLaxMethodUnsafe, SameSiteStrict},
        NetworkSameSiteContextDowngradeType::StrictToLax,
        NetworkSiteContext,
    };

    let start = Url::parse("https://example.test/start")?;
    let end = Url::parse("https://example.test/end")?;
    for (middle, post_context, get_context) in [
        (
            "https://other.test/middle",
            SameSiteLaxMethodUnsafe,
            SameSiteLax,
        ),
        ("http://example.test/middle", SameSiteStrict, SameSiteStrict),
    ] {
        let middle = Url::parse(middle)?;
        for force_cross_site in [false, true] {
            let mut request = Request::new("POST", start.as_str(), None, vec![])?
                .with_top_level_navigation_cookie_context()
                .with_initiator_url(&start);
            if force_cross_site {
                request = request.with_cross_site_cookie_context();
            }
            request.follow_redirect(redirect(&start, &middle, 307))?;
            let expected = if force_cross_site {
                NetworkSiteContext::cross_site()
            } else {
                NetworkSiteContext::new(post_context, SameSiteLaxMethodUnsafe)
            };
            assert_eq!(request.cookie_context.site_context, expected);
            request.follow_redirect(redirect(&middle, &end, 302))?;
            assert_eq!(request.method, "GET");
            assert!(request.cookie_context.is_method_safe);
            let expected = if force_cross_site {
                NetworkSiteContext::cross_site()
            } else {
                NetworkSiteContext::new(get_context, SameSiteLax)
            };
            assert_eq!(request.cookie_context.site_context, expected);
            let get_metadata =
                NetworkCookieRequestContext::top_level_navigation("GET").site_context_metadata;
            let metadata = request.cookie_context.site_context_metadata;
            assert_eq!(
                metadata.context.http_method,
                get_metadata.context.http_method
            );
            assert_eq!(
                metadata.schemeful_context.http_method,
                get_metadata.schemeful_context.http_method
            );
            if !force_cross_site {
                assert_eq!(
                    metadata.context.downgrade_type,
                    (get_context == SameSiteLax).then_some(StrictToLax)
                );
                assert_eq!(metadata.schemeful_context.downgrade_type, Some(StrictToLax));
                assert!(metadata.schemeful_context.downgraded_by_cross_site_redirect);
            }
        }
    }
    Ok(())
}

fn assert_browser_redirect_cookie_context(force_cross_site: bool) -> Result<()> {
    use moli_cookie_jar::test_support::{NetworkSameSiteContext, NetworkSiteContext};

    let start = Url::parse("https://example.test/start")?;
    let middle = Url::parse("https://other.test/middle")?;
    let end = Url::parse("https://example.test/end")?;
    let mut request = Request::new_browser(
        "POST",
        start.clone(),
        None,
        vec![],
        moli_url::WebOrigin::from_url(&start),
    )
    .with_top_level_navigation_cookie_context();
    if force_cross_site {
        request = request.with_cross_site_cookie_context();
    }
    request.follow_redirect(redirect(&start, &middle, 307))?;
    let expected = if force_cross_site {
        NetworkSiteContext::cross_site()
    } else {
        NetworkSiteContext::new(
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe,
        )
    };
    assert_eq!(request.network_cookie_context().site_context, expected);
    request.follow_redirect(redirect(&middle, &end, 302))?;
    let expected = if force_cross_site {
        NetworkSiteContext::cross_site()
    } else {
        NetworkSiteContext::new(
            NetworkSameSiteContext::SameSiteLax,
            NetworkSameSiteContext::SameSiteLax,
        )
    };
    assert_eq!(request.network_cookie_context().site_context, expected);
    assert_eq!(request.cookie_context.site_context, expected);
    Ok(())
}

#[test]
fn browser_redirect_cookie_context_keeps_the_origin_only_redirect_chain() -> Result<()> {
    assert_browser_redirect_cookie_context(false)
}

#[test]
fn browser_redirect_cookie_context_keeps_explicit_cross_site_restrictions() -> Result<()> {
    assert_browser_redirect_cookie_context(true)
}

#[test]
fn network_cookie_context_uses_the_current_overridden_request_method() -> Result<()> {
    let request_url = Url::parse("https://other.test/target")?;
    let origin = Url::parse("https://example.test/page")?;
    let mut request = Request::new_browser(
        "POST",
        request_url,
        None,
        vec![],
        moli_url::WebOrigin::from_url(&origin),
    )
    .with_top_level_navigation_cookie_context();
    request.method = "GET".to_owned();
    let actual = request.network_cookie_context();
    let expected = NetworkCookieRequestContext::top_level_navigation("GET");
    assert!(actual.is_method_safe);
    assert_eq!(
        actual.site_context_metadata.context.http_method,
        expected.site_context_metadata.context.http_method
    );
    assert_eq!(
        actual.site_context_metadata.schemeful_context.http_method,
        expected.site_context_metadata.schemeful_context.http_method
    );
    Ok(())
}

#[test]
fn browser_origin_rebinding_recomputes_provisional_cross_site_context() -> Result<()> {
    use moli_cookie_jar::test_support::NetworkSiteContext;

    let origin = Url::parse("https://example.test/page")?;
    let target = Url::parse("https://example.test/data")?;
    let base = Url::parse("https://other.test/base/")?;
    let request = Request::new_browser(
        "GET",
        target,
        None,
        vec![],
        moli_url::WebOrigin::from_url(&origin),
    )
    .with_initiator_url(&base)
    .with_browser_site_context(
        moli_cookie_jar::BrowserCookieFacadeContext::default()
            .with_site_for_cookies_url(&origin)
            .with_top_frame_origin_url(&origin),
    );
    assert!(request.cookie_context.site_context.is_cross_site());
    assert_eq!(
        request.network_cookie_context().site_context,
        NetworkSiteContext::same_site()
    );
    Ok(())
}

#[test]
fn browser_scope_refresh_keeps_followed_redirect_limits() -> Result<()> {
    use moli_cookie_jar::test_support::{NetworkSameSiteContext::SameSiteLax, NetworkSiteContext};

    let start = Url::parse("https://example.test/start")?;
    let middle = Url::parse("https://other.test/middle")?;
    let end = Url::parse("https://example.test/end")?;
    let mut request = Request::new_browser(
        "POST",
        start.clone(),
        None,
        vec![],
        moli_url::WebOrigin::from_url(&start),
    )
    .with_top_level_navigation_cookie_context();
    request.follow_redirect(redirect(&start, &middle, 307))?;
    request.follow_redirect(redirect(&middle, &end, 302))?;
    request = request.with_browser_site_context(
        moli_cookie_jar::BrowserCookieFacadeContext::default()
            .with_site_for_cookies_url(&start)
            .with_top_frame_origin_url(&start),
    );
    assert_eq!(
        request.network_cookie_context().site_context,
        NetworkSiteContext::new(SameSiteLax, SameSiteLax)
    );
    Ok(())
}

#[test]
fn subframe_navigation_preserves_explicit_cookie_constraints() -> Result<()> {
    let url = Url::parse("https://example.test/page")?;
    let request = Request::new_browser(
        "GET",
        url.clone(),
        None,
        vec![],
        moli_url::WebOrigin::from_url(&url),
    )
    .with_cross_site_cookie_context()
    .with_subframe_navigation_cookie_context();
    assert!(
        request
            .network_cookie_context()
            .site_context
            .is_cross_site()
    );
    Ok(())
}

#[tokio::test]
async fn cached_redirect_cookies_keep_the_rewritten_method_in_streaming_transports() -> Result<()> {
    // Buffered responses cache the final result; only streaming transports cache hops.
    for transport in [Transport::Html, Transport::Raw] {
        let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("target"); 2]);
        let mut target_url = Url::parse(&target.url_path("/final"))?;
        target_url.set_host(Some("other.test"))?;
        let middle = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(302, "Found")
                .with_header("Location", target_url.as_str())
                .with_header("Cache-Control", "max-age=3600"),
        ]);
        let mut middle_url = Url::parse(&middle.url_path("/middle"))?;
        middle_url.set_host(Some("other.test"))?;
        let source = ScriptedHttpServer::spawn(vec![
            ScriptedResponse::status(303, "See Other").with_header("Location", middle_url.as_str()),
        ]);
        let source_url = Url::parse(&source.url())?;
        let cache_dir = unique_test_cache_dir();
        let mut config = FetchConfig::default();
        config.set_http_cache_dir(Some(cache_dir.display().to_string()));
        config.set_http_no_proxy(Some("*".to_owned()));
        config.set_http_host_resolve(vec![
            format!("other.test:{}:127.0.0.1", target_url.port().unwrap()),
            format!("other.test:{}:127.0.0.1", middle_url.port().unwrap()),
        ]);
        let cookies = new_shared_browser_cookie_store();
        // The intermediate URL has no Cookie header, so its redirect can be cached.
        seed_redirect_cookies(&cookies, &target_url, "/final");
        let client = FetchClient::new(&config, cookies);
        transport
            .fetch(
                &client,
                Request::get(middle_url.as_str())?.with_initiator_url(&source_url),
            )
            .await?;
        transport
            .fetch(
                &client,
                Request::new(
                    "POST",
                    source_url.as_str(),
                    Some("payload".to_owned()),
                    vec![],
                )?
                .with_top_level_navigation_cookie_context()
                .with_initiator_url(&source_url),
            )
            .await?;
        let source_requests = source.requests();
        let middle_requests = middle.requests();
        let target_requests = target.requests();
        source.shutdown();
        middle.shutdown();
        target.shutdown();
        fs::remove_dir_all(cache_dir)?;
        assert_eq!(source_requests.len(), 1, "{transport:?}");
        assert!(source_requests[0].starts_with("POST "));
        assert_eq!(
            middle_requests.len(),
            1,
            "redirect must be served from cache: {transport:?}"
        );
        assert_eq!(target_requests.len(), 2, "{transport:?}");
        for request in target_requests {
            assert!(request.starts_with("GET /final "));
            assert_eq!(
                request_cookies(&request),
                vec!["aged=1", "lax=1", "unspecified=1"],
                "{transport:?}: {request}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn redirected_navigation_keeps_cookie_method_allowance_in_every_transport() -> Result<()> {
    let cases: [(&str, bool, bool, &[&str]); 4] = [
        ("GET", true, false, &["lax=1", "unspecified=1"]),
        ("POST", true, false, &["unspecified=1"]),
        ("GET", false, false, &[]),
        ("POST", true, true, &[]),
    ];
    for transport in Transport::ALL {
        for (method, navigation, force_cross_site, expected) in cases {
            let target = ScriptedHttpServer::spawn(vec![ScriptedResponse::ok("target")]);
            let mut target_url = Url::parse(&target.url())?;
            target_url.set_host(Some("other.test"))?;
            let source = ScriptedHttpServer::spawn(vec![
                ScriptedResponse::status(307, "Temporary Redirect")
                    .with_header("Location", target_url.as_str()),
            ]);
            let source_url = Url::parse(&source.url())?;
            let mut config = FetchConfig::default();
            config.set_http_no_proxy(Some("*".to_owned()));
            config.set_http_host_resolve(vec![format!(
                "other.test:{}:127.0.0.1",
                target_url.port().unwrap()
            )]);
            let cookies = new_shared_browser_cookie_store();
            cookies
                .lock()
                .expect("cookie store is available")
                .store_response_headers(
                    &target_url,
                    &[
                        (
                            "set-cookie".to_owned(),
                            b"strict=1; SameSite=Strict; Path=/".to_vec(),
                        ),
                        (
                            "set-cookie".to_owned(),
                            b"lax=1; SameSite=Lax; Path=/".to_vec(),
                        ),
                        ("set-cookie".to_owned(), b"unspecified=1; Path=/".to_vec()),
                    ],
                );
            let client = FetchClient::new(&config, cookies);
            let mut request = Request::new(method, source_url.as_str(), None, vec![])?;
            if navigation {
                request = request.with_top_level_navigation_cookie_context();
            }
            request = request
                .with_initiator_url(&source_url)
                .with_site_for_cookies_url(&source_url)
                .with_top_frame_origin_url(&source_url);
            if force_cross_site {
                request = request.with_cross_site_cookie_context();
            }
            transport.fetch(&client, request).await?;
            let source_requests = source.requests();
            let target_requests = target.requests();
            source.shutdown();
            target.shutdown();
            assert_eq!(source_requests.len(), 1);
            assert_eq!(target_requests.len(), 1);
            assert!(
                target_requests[0].starts_with(&format!("{method} {} ", target_url.path())),
                "unexpected redirected request: {}",
                target_requests[0],
            );
            let mut actual: Vec<_> = target_requests[0]
                .lines()
                .filter_map(|line| line.split_once(':'))
                .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
                .flat_map(|(_, value)| value.split(';').map(str::trim))
                .collect();
            actual.sort_unstable();
            let mut expected = expected.to_vec();
            expected.sort_unstable();
            assert_eq!(
                actual, expected,
                "{transport:?}, {method}, navigation={navigation}, force_cross_site={force_cross_site}: {}",
                target_requests[0],
            );
        }
    }
    Ok(())
}
