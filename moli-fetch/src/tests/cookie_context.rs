use moli_cookie_jar::test_support::{
    BrowserCookieStore, NetworkSameSiteContext, NetworkSameSiteContextDowngradeType,
    NetworkSameSiteRedirectType, NetworkSiteContext,
};
use moli_cookie_jar::{
    BrowserCookieFacadeContext, NetworkCookieRequestContext, NetworkStorageAccessStatus,
    StoredCookieSameSiteContextDowngradeType, StoredCookieSameSiteRedirectType,
    StoredCookieWarningReason, advance_cookie_request_context, new_shared_browser_cookie_store,
};
use url::Url;

use crate::{Request, RequestCredentialsMode, cookie_header_for_request};

#[test]
fn request_get_uses_top_level_navigation_cookie_context() {
    let request = Request::get("https://example.com/app").unwrap();

    assert_eq!(
        request.cookie_context,
        NetworkCookieRequestContext::top_level_navigation("GET")
    );
}

#[test]
fn request_new_uses_subresource_cookie_context_and_tracks_method_safety() {
    let safe_request = Request::new("HEAD", "https://example.com/app", None, vec![]).unwrap();
    let unsafe_request = Request::new(
        "POST",
        "https://example.com/app",
        Some("body".to_owned()),
        vec![],
    )
    .unwrap();

    assert_eq!(
        safe_request.cookie_context,
        NetworkCookieRequestContext::subresource("HEAD")
    );
    assert_eq!(
        unsafe_request.cookie_context,
        NetworkCookieRequestContext::subresource("POST")
    );
}

#[test]
fn browser_site_context_overlay_downgrades_but_never_relaxes_same_site_context() {
    let top_url = Url::parse("https://top.test/page").unwrap();
    let child_url = Url::parse("https://child.test/worker.js").unwrap();
    let browser_context = BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&top_url)
        .with_top_frame_origin_url(&top_url);

    let computed = Request::new("GET", child_url.as_str(), None, vec![])
        .unwrap()
        .with_initiator_url(&child_url)
        .with_browser_site_context(browser_context.clone());
    assert!(
        computed.cookie_context.site_context.is_cross_site(),
        "the inherited top-frame site must downgrade a same-origin child request"
    );

    let same_site_browser_context = BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&child_url)
        .with_top_frame_origin_url(&child_url);
    let explicitly_cross_site = Request::new("GET", child_url.as_str(), None, vec![])
        .unwrap()
        .with_initiator_url(&child_url)
        .with_cross_site_cookie_context()
        .with_browser_site_context(same_site_browser_context);
    assert!(
        explicitly_cross_site
            .cookie_context
            .site_context
            .is_cross_site(),
        "a worker sameSiteCookies=none restriction must survive context propagation"
    );
}

#[test]
fn request_credentials_mode_controls_cross_origin_cookie_access() {
    let initiator_url = Url::parse("https://example.com/app/page.html").unwrap();
    let same_origin_url = Url::parse("https://example.com/api/data").unwrap();
    let cross_origin_url = Url::parse("https://example.com:9443/api/data").unwrap();

    let default_request = Request::new("GET", same_origin_url.as_str(), None, vec![])
        .unwrap()
        .with_initiator_url(&initiator_url);
    assert_eq!(
        default_request.credentials_mode,
        RequestCredentialsMode::Include
    );
    assert!(default_request.allows_credentials_for_url(&cross_origin_url));

    let same_origin_request = Request::new("GET", same_origin_url.as_str(), None, vec![])
        .unwrap()
        .with_initiator_url(&initiator_url)
        .with_credentials_mode(RequestCredentialsMode::SameOrigin);
    assert!(same_origin_request.allows_credentials_for_url(&same_origin_url));
    assert!(!same_origin_request.allows_credentials_for_url(&cross_origin_url));

    let omit_request = Request::new("GET", same_origin_url.as_str(), None, vec![])
        .unwrap()
        .with_initiator_url(&initiator_url)
        .with_credentials_mode(RequestCredentialsMode::Omit);
    assert!(!omit_request.allows_credentials_for_url(&same_origin_url));
}

#[test]
fn opaque_request_origin_disallows_same_origin_credentials_for_same_url_origin() {
    let document_url = Url::parse("https://example.com/app/page.html").unwrap();
    let request_url = Url::parse("https://example.com/api/data").unwrap();
    let request = Request::new("GET", request_url.as_str(), None, vec![])
        .unwrap()
        .with_initiator_url(&document_url)
        .with_request_origin(moli_url::WebOrigin::Opaque)
        .with_credentials_mode(RequestCredentialsMode::SameOrigin);

    assert!(!request.allows_credentials_for_url(&request_url));
}

#[test]
fn explicit_same_site_override_sets_both_site_context_tracks() {
    let context = NetworkCookieRequestContext::subresource("GET")
        .with_same_site_context(NetworkSameSiteContext::CrossSite);

    assert_eq!(
        context.site_context.context,
        NetworkSameSiteContext::CrossSite
    );
    assert_eq!(
        context.site_context.schemeful_context,
        NetworkSameSiteContext::CrossSite
    );
}

#[test]
fn request_cookie_header_respects_explicit_top_level_lax_navigation_context() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://example.com/app/index.html").unwrap();
    let request = Request::get("https://example.com/app/panel")
        .unwrap()
        .with_cookie_context(
            NetworkCookieRequestContext::top_level_navigation("GET").with_site_context(
                NetworkSiteContext::new(
                    NetworkSameSiteContext::SameSiteLax,
                    NetworkSameSiteContext::SameSiteLax,
                ),
            ),
        );

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        cookie_header_for_request(&cookie_store, &request.url, request.cookie_context).unwrap(),
        Some("lax=1".to_owned())
    );
}

#[test]
fn request_cookie_header_respects_explicit_cross_site_context_without_lax_upgrade() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://example.com/app/index.html").unwrap();
    let request = Request::get("https://example.com/app/panel")
        .unwrap()
        .with_cookie_context(
            NetworkCookieRequestContext::top_level_navigation("GET")
                .with_site_context(NetworkSiteContext::cross_site()),
        );

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "none=1; Path=/app; Secure; SameSite=None".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        cookie_header_for_request(&cookie_store, &request.url, request.cookie_context).unwrap(),
        Some("none=1".to_owned())
    );
}

#[test]
fn request_with_initiator_url_marks_cross_site_subresource_requests() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://example.com/app/index.html").unwrap();
    let request = Request::new("GET", "https://example.com/app/panel", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://other.test/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "none=1; Path=/app; Secure; SameSite=None".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        cookie_header_for_request(&cookie_store, &request.url, request.cookie_context).unwrap(),
        Some("none=1".to_owned())
    );
}

#[test]
fn request_with_initiator_url_treats_same_scheme_subdomains_as_same_site() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://sub.example.com/app/index.html").unwrap();
    let request = Request::new("GET", "https://sub.example.com/app/panel", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[(
                "set-cookie".to_owned(),
                "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
            )],
        );
    }

    assert_eq!(
        cookie_header_for_request(&cookie_store, &request.url, request.cookie_context).unwrap(),
        Some("strict=1".to_owned())
    );
}

#[test]
fn request_with_initiator_url_treats_same_scheme_sibling_subdomains_as_same_site() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://api.example.com/app/index.html").unwrap();
    let request = Request::new("GET", "https://api.example.com/app/panel", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://www.example.com/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[(
                "set-cookie".to_owned(),
                "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
            )],
        );
    }

    assert_eq!(
        request.cookie_context.site_context.context,
        NetworkSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        request.cookie_context.site_context.schemeful_context,
        NetworkSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        cookie_header_for_request(&cookie_store, &request.url, request.cookie_context).unwrap(),
        Some("strict=1".to_owned())
    );
}

#[test]
fn request_with_initiator_url_respects_public_suffix_boundaries() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://foo.co.uk/app/index.html").unwrap();
    let request = Request::new("GET", "https://foo.co.uk/app/panel", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://bar.co.uk/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        request.cookie_context.site_context.context,
        NetworkSameSiteContext::CrossSite
    );
    assert_eq!(
        request.cookie_context.site_context.schemeful_context,
        NetworkSameSiteContext::CrossSite
    );
    assert_eq!(
        cookie_header_for_request(&cookie_store, &request.url, request.cookie_context).unwrap(),
        None
    );
}

#[test]
fn request_with_initiator_url_treats_multi_label_registrable_domains_as_same_site() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://api.example.co.uk/app/index.html").unwrap();
    let request = Request::new("GET", "https://api.example.co.uk/app/panel", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://www.example.co.uk/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[(
                "set-cookie".to_owned(),
                "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
            )],
        );
    }

    assert_eq!(
        request.cookie_context.site_context.context,
        NetworkSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        cookie_header_for_request(&cookie_store, &request.url, request.cookie_context).unwrap(),
        Some("strict=1".to_owned())
    );
}

#[test]
fn request_with_initiator_url_tracks_schemeless_and_schemeful_site_context_separately() {
    let request = Request::new("GET", "https://example.com/app/panel", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("http://example.com/index.html").unwrap());

    assert_eq!(
        request.cookie_context.site_context.context,
        NetworkSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        request.cookie_context.site_context.schemeful_context,
        NetworkSameSiteContext::CrossSite
    );
}

#[test]
fn request_with_site_for_cookies_url_uses_explicit_browser_context_for_same_site() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://example.com/app/index.html").unwrap();
    let request = Request::new("GET", "https://example.com/app/panel", None, vec![])
        .unwrap()
        .with_site_for_cookies_url(&Url::parse("https://other.test/frame.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "none=1; Path=/app; Secure; SameSite=None".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        request
            .cookie_context
            .browser_context
            .site_for_cookies_url
            .as_ref()
            .map(Url::as_str),
        Some("https://other.test/frame.html")
    );
    assert_eq!(
        cookie_header_for_request(
            &cookie_store,
            &request.url,
            request.effective_cookie_context(&request.url),
        )
        .unwrap(),
        Some("none=1".to_owned())
    );
}

#[test]
fn request_with_top_frame_origin_url_falls_back_when_site_for_cookies_is_absent() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://other.test/app/index.html").unwrap();
    let request = Request::get("https://other.test/app/panel")
        .unwrap()
        .with_top_frame_origin_url(&Url::parse("https://example.com/root").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        request
            .cookie_context
            .browser_context
            .top_frame_origin_url
            .as_ref()
            .map(Url::as_str),
        Some("https://example.com/root")
    );
    assert_eq!(
        cookie_header_for_request(
            &cookie_store,
            &request.url,
            request.effective_cookie_context(&request.url),
        )
        .unwrap(),
        Some("lax=1".to_owned())
    );
}

#[test]
fn request_with_storage_access_status_preserves_browser_context() {
    let request = Request::new("GET", "https://example.com/app/panel", None, vec![])
        .unwrap()
        .with_storage_access_status(NetworkStorageAccessStatus::Granted);

    assert_eq!(
        request.cookie_context.browser_context.storage_access_status,
        NetworkStorageAccessStatus::Granted
    );
}

#[test]
fn request_with_cross_scheme_initiator_uses_schemeful_cross_site_semantics() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://example.com/app/index.html").unwrap();
    let request = Request::new("GET", "https://example.com/app/panel", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("http://example.com/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "none=1; Path=/app; Secure; SameSite=None".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        cookie_header_for_request(
            &cookie_store,
            &request.url,
            request.effective_cookie_context(&request.url),
        )
        .unwrap(),
        Some("none=1".to_owned())
    );
}

#[test]
fn request_effective_cookie_context_recomputes_same_site_for_redirect_targets() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://other.test/app/index.html").unwrap();
    let redirected_url = Url::parse("https://other.test/app/panel").unwrap();
    let request = Request::new("GET", "https://example.com/start", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "none=1; Path=/app; Secure; SameSite=None".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        cookie_header_for_request(
            &cookie_store,
            &redirected_url,
            request.effective_cookie_context(&redirected_url),
        )
        .unwrap(),
        Some("none=1".to_owned())
    );
}

#[test]
fn request_effective_cookie_context_marks_cross_site_redirect_downgrade_in_report() {
    let redirected_url = Url::parse("https://other.test/app/panel").unwrap();
    let response_url = Url::parse("https://other.test/app/index.html").unwrap();
    let request = Request::new("GET", "https://example.com/start", None, vec![])
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/index.html").unwrap());
    let mut jar = BrowserCookieStore::default();

    jar.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = jar.cookie_access_report_for_request(
        &redirected_url,
        request.effective_cookie_context(&redirected_url),
    );
    let strict = report
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name == "strict")
        .expect("strict cookie should be excluded after redirect downgrade");

    assert_eq!(
        strict.warning_reasons,
        vec![StoredCookieWarningReason::SameSiteContextDowngradedByRedirect]
    );
    assert_eq!(
        strict.same_site_context_downgrade_type,
        Some(StoredCookieSameSiteContextDowngradeType::StrictToCross)
    );
    assert_eq!(
        strict.schemeful_same_site_context_downgrade_type,
        Some(StoredCookieSameSiteContextDowngradeType::StrictToCross)
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        strict.same_site_context,
        moli_cookie_jar::StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        moli_cookie_jar::StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn request_effective_cookie_context_records_strict_to_lax_redirect_downgrade() {
    let request = Request::get("https://same.test/app/panel")
        .unwrap()
        .with_initiator_url(&Url::parse("https://same.test/index.html").unwrap());

    let redirected_context =
        request.effective_cookie_context(&Url::parse("https://cross.test/app/panel").unwrap());

    assert_eq!(
        redirected_context
            .site_context_metadata
            .schemeful_context
            .redirect_type,
        NetworkSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        redirected_context
            .site_context_metadata
            .context
            .redirect_type,
        NetworkSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        redirected_context
            .site_context_metadata
            .schemeful_context
            .downgrade_type,
        Some(NetworkSameSiteContextDowngradeType::StrictToLax)
    );
    assert_eq!(
        redirected_context
            .site_context_metadata
            .context
            .downgrade_type,
        Some(NetworkSameSiteContextDowngradeType::StrictToLax)
    );
    assert_eq!(
        redirected_context.site_context.schemeful_context,
        NetworkSameSiteContext::SameSiteLax
    );
}

#[test]
fn request_effective_cookie_context_tracks_schemeful_only_redirect_downgrade() {
    let request = Request::get("https://example.com/app/panel")
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/index.html").unwrap());

    let redirected_context =
        request.effective_cookie_context(&Url::parse("http://example.com/app/panel").unwrap());

    assert_eq!(
        redirected_context.site_context.context,
        NetworkSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        redirected_context.site_context.schemeful_context,
        NetworkSameSiteContext::SameSiteLax
    );
    assert_eq!(
        redirected_context
            .site_context_metadata
            .context
            .downgrade_type,
        None
    );
    assert_eq!(
        redirected_context
            .site_context_metadata
            .schemeful_context
            .downgrade_type,
        Some(NetworkSameSiteContextDowngradeType::StrictToLax)
    );
    assert_eq!(
        redirected_context
            .site_context_metadata
            .context
            .redirect_type,
        NetworkSameSiteRedirectType::AllSameSiteRedirect
    );
    assert_eq!(
        redirected_context
            .site_context_metadata
            .schemeful_context
            .redirect_type,
        NetworkSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn redirect_chain_cookie_context_preserves_cross_site_downgrade_across_later_same_site_hops() {
    let cookie_store = new_shared_browser_cookie_store();
    let first_redirect_url = Url::parse("https://cross.test/hop").unwrap();
    let final_url = Url::parse("https://same.test/final").unwrap();
    let request = Request::new("GET", "https://same.test/start", None, Vec::new())
        .unwrap()
        .with_initiator_url(&Url::parse("https://same.test/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &final_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "none=1; Path=/; Secure; SameSite=None".to_owned(),
                ),
            ],
        );
    }

    let first_hop_context = advance_cookie_request_context(
        request.cookie_context.clone(),
        &request.url,
        &first_redirect_url,
    );
    let final_hop_context =
        advance_cookie_request_context(first_hop_context, &request.url, &final_url);

    assert_eq!(
        cookie_header_for_request(&cookie_store, &final_url, final_hop_context).unwrap(),
        Some("none=1".to_owned())
    );
}

#[test]
fn top_level_request_with_initiator_url_uses_cross_site_lax_navigation_semantics() {
    let cookie_store = new_shared_browser_cookie_store();
    let response_url = Url::parse("https://other.test/app/index.html").unwrap();
    let request = Request::get("https://other.test/app/panel")
        .unwrap()
        .with_initiator_url(&Url::parse("https://example.com/index.html").unwrap());

    {
        let mut jar = cookie_store.lock();
        jar.store_response_headers(
            &response_url,
            &[
                (
                    "set-cookie".to_owned(),
                    "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
                ),
                (
                    "set-cookie".to_owned(),
                    "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
                ),
            ],
        );
    }

    assert_eq!(
        cookie_header_for_request(
            &cookie_store,
            &request.url,
            request.effective_cookie_context(&request.url),
        )
        .unwrap(),
        Some("lax=1".to_owned())
    );
}

#[test]
fn top_level_post_with_initiator_url_uses_lax_method_unsafe_context() {
    let request = Request::new("POST", "https://other.test/app/panel", None, vec![])
        .unwrap()
        .with_top_level_navigation_cookie_context()
        .with_initiator_url(&Url::parse("https://example.com/index.html").unwrap());

    let effective_context = request.effective_cookie_context(&request.url);

    assert_eq!(
        effective_context.site_context.schemeful_context,
        NetworkSameSiteContext::SameSiteLaxMethodUnsafe
    );
}
