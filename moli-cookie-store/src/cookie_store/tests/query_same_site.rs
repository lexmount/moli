use super::*;

#[test]
fn cross_site_subresource_query_excludes_explicit_lax_and_strict_same_site_cookies() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "lax=1; Path=/foo; Secure; SameSite=Lax",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "none=1; Path=/foo; Secure; SameSite=None",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "unspecified=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_query_result(&QueryContext::http_cross_site(&request_url));
    assert_eq!(result.included_cookies.len(), 2);
    assert!(result
        .included_cookies
        .iter()
        .any(|cookie| cookie.name() == "none"));
    assert!(result
        .included_cookies
        .iter()
        .any(|cookie| cookie.name() == "unspecified"));
    assert_eq!(result.excluded_cookies.len(), 2);
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "strict" && cookie.reason == CookieExclusionReason::SameSiteStrict
    }));
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "lax" && cookie.reason == CookieExclusionReason::SameSiteLax
    }));
}

#[test]
fn cross_site_top_level_safe_query_allows_lax_but_excludes_strict() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "lax=1; Path=/foo; Secure; SameSite=Lax",
        &InsertContext::http(&request_url),
    ));

    let result =
        store.get_cookie_query_result(&QueryContext::http_cross_site_top_level(&request_url));
    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(result.included_cookies[0].name(), "lax");
    assert_eq!(result.excluded_cookies.len(), 1);
    assert_eq!(result.excluded_cookies[0].cookie.name(), "strict");
    assert_eq!(
        result.excluded_cookies[0].reason,
        CookieExclusionReason::SameSiteStrict
    );
}

#[test]
fn cross_site_top_level_unsafe_query_excludes_explicit_lax_and_strict() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "lax=1; Path=/foo; Secure; SameSite=Lax",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_query_result(&QueryContext::http_cross_site_top_level_unsafe(
        &request_url,
    ));
    assert!(result.included_cookies.is_empty());
    assert_eq!(result.excluded_cookies.len(), 2);
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "strict" && cookie.reason == CookieExclusionReason::SameSiteStrict
    }));
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "lax" && cookie.reason == CookieExclusionReason::SameSiteLax
    }));
}

#[test]
fn same_site_http_query_still_includes_explicit_lax_and_strict_cookies() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "lax=1; Path=/foo; Secure; SameSite=Lax",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_query_result(&QueryContext::http(&request_url));
    assert_eq!(result.included_cookies.len(), 2);
    assert!(result.excluded_cookies.is_empty());
}

#[test]
fn document_query_ignores_same_site_request_context() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::document(&request_url);
    context.same_site_context = SameSiteContext::cross_site();
    context.request_type = HttpRequestType::TopLevelNavigation;
    context.is_method_safe = false;
    let result = store.get_cookie_query_result(&context);
    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(result.included_cookies[0].name(), "strict");
    assert!(result.excluded_cookies.is_empty());
}

#[test]
fn schemeful_cross_site_query_uses_stricter_same_site_relation() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "lax=1; Path=/foo; Secure; SameSite=Lax",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "none=1; Path=/foo; Secure; SameSite=None",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::http(&request_url);
    context.same_site_context = SameSiteContext::new(
        SameSiteRequestContext::SameSiteStrict,
        SameSiteRequestContext::CrossSite,
    );
    let result = store.get_cookie_query_result(&context);

    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(result.included_cookies[0].name(), "none");
    assert_eq!(result.excluded_cookies.len(), 2);
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "strict" && cookie.reason == CookieExclusionReason::SameSiteStrict
    }));
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "lax" && cookie.reason == CookieExclusionReason::SameSiteLax
    }));
}

#[test]
fn access_query_result_reports_schemeful_same_site_warning_when_contexts_diverge() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "none=1; Path=/foo; Secure; SameSite=None",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::http(&request_url);
    context.same_site_context = SameSiteContext::new(
        SameSiteRequestContext::SameSiteStrict,
        SameSiteRequestContext::CrossSite,
    );

    let result = store.get_cookie_access_query_result(&context);

    let strict = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "strict")
        .expect("strict cookie should be excluded");
    assert_eq!(
        strict.access_result.status.exclusion_reasons,
        vec![CookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.access_result.status.warning_reasons,
        vec![CookieWarningReason::StrictCrossDowngradeStrictSameSite]
    );
    assert_eq!(
        strict.access_result.effective_same_site,
        CookieEffectiveSameSite::Strict
    );
    assert_eq!(
        strict.access_result.same_site_context.context,
        SameSiteRequestContext::SameSiteStrict
    );
    assert_eq!(
        strict.access_result.same_site_context.schemeful_context,
        SameSiteRequestContext::CrossSite
    );

    let none = result
        .included_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "none")
        .expect("none cookie should remain included");
    assert!(none.access_result.status.is_included());
    assert!(none.access_result.status.warning_reasons.is_empty());
    assert_eq!(
        none.access_result.effective_same_site,
        CookieEffectiveSameSite::NoRestriction
    );
}

#[test]
fn access_query_result_reports_lax_schemeful_same_site_warning_when_contexts_diverge() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "lax=1; Path=/foo; Secure; SameSite=Lax",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::http(&request_url);
    context.same_site_context = SameSiteContext::new(
        SameSiteRequestContext::SameSiteStrict,
        SameSiteRequestContext::CrossSite,
    );
    context.same_site_context_metadata = SameSiteContextMetadata::schemeful_only(
        false,
        Some(SameSiteContextDowngradeType::StrictToCross),
    );

    let result = store.get_cookie_access_query_result(&context);
    let lax = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "lax")
        .expect("lax cookie should be excluded");

    assert_eq!(
        lax.access_result.status.exclusion_reasons,
        vec![CookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.access_result.status.warning_reasons,
        vec![CookieWarningReason::StrictCrossDowngradeLaxSameSite]
    );
    assert_eq!(
        lax.access_result.effective_same_site,
        CookieEffectiveSameSite::Lax
    );
}

#[test]
fn access_query_result_reports_effective_same_site_for_lax_cookie() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "lax=1; Path=/foo; Secure; SameSite=Lax",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_access_query_result(&QueryContext::http(&request_url));
    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(
        result.included_cookies[0].access_result.effective_same_site,
        CookieEffectiveSameSite::Lax
    );
    assert!(!result.included_cookies[0]
        .access_result
        .status
        .has_warnings());
}

#[test]
fn access_query_result_reports_access_semantics_for_explicit_same_site_cookie() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_access_query_result(&QueryContext::http(&request_url));
    let strict = result
        .included_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "strict")
        .expect("strict cookie should be included");

    assert_eq!(
        strict.access_result.access_semantics,
        CookieAccessSemantics::NonLegacy
    );
    assert_eq!(
        strict.access_result.scope_semantics,
        CookieScopeSemantics::Unknown
    );
    assert!(strict.access_result.is_allowed_to_access_secure_cookies);
}

#[test]
fn access_query_result_reports_unknown_access_semantics_for_unspecified_same_site_cookie() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_access_query_result(&QueryContext::http(&request_url));
    let sid = result
        .included_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "sid")
        .expect("cookie should be included");

    assert_eq!(
        sid.access_result.effective_same_site,
        CookieEffectiveSameSite::NoRestriction
    );
    assert_eq!(
        sid.access_result.access_semantics,
        CookieAccessSemantics::Unknown
    );
}

#[test]
fn access_query_result_reports_secure_cookie_capability_for_insecure_context() {
    let mut store = CookieStore::default();
    let secure_url = test_utils::url("https://example.com/foo/bar");
    let insecure_url = test_utils::url("http://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&secure_url),
    ));

    let result = store.get_cookie_access_query_result(&QueryContext::http(&insecure_url));
    let sid = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "sid")
        .expect("secure cookie should be excluded");

    assert_eq!(
        sid.access_result.status.exclusion_reasons,
        vec![
            CookieExclusionReason::SecureOnly,
            CookieExclusionReason::SchemeMismatch,
        ]
    );
    assert!(!sid.access_result.is_allowed_to_access_secure_cookies);
}

#[test]
fn access_query_result_warns_when_secure_cookie_is_granted_on_localhost_http() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("http://localhost/app/index.html");

    let result = store.set_response_cookie_str_with_access_result(
        "sid=1; Path=/app; Secure",
        &InsertContext::http(&request_url),
    );
    assert!(result.is_accepted());

    let query = store.get_cookie_access_query_result(&QueryContext::http(&request_url));
    let sid = query
        .included_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "sid")
        .expect("secure cookie should be included for localhost http");

    assert!(sid.access_result.status.is_included());
    assert_eq!(
        sid.access_result.status.warning_reasons,
        vec![CookieWarningReason::SecureAccessGrantedNonCryptographic]
    );
    assert!(sid.access_result.is_allowed_to_access_secure_cookies);
}

#[test]
fn access_query_result_does_not_warn_when_secure_cookie_is_accessed_over_wss() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("wss://example.com/app/index.html");

    let result = store.set_response_cookie_str_with_access_result(
        "sid=1; Path=/app; Secure",
        &InsertContext::http(&request_url),
    );
    assert!(result.is_accepted());

    let query = store.get_cookie_access_query_result(&QueryContext::http(&request_url));
    let sid = query
        .included_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "sid")
        .expect("secure cookie should be included for wss");

    assert!(sid.access_result.status.is_included());
    assert!(sid.access_result.status.warning_reasons.is_empty());
    assert!(sid.access_result.is_allowed_to_access_secure_cookies);
}

#[test]
fn access_query_result_reports_same_site_redirect_downgrade_warning() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://other.test/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::http_cross_site(&request_url);
    context.same_site_context_metadata = SameSiteContextMetadata::schemeful_only(
        true,
        Some(SameSiteContextDowngradeType::StrictToCross),
    );
    let result = store.get_cookie_access_query_result(&context);
    let strict = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "strict")
        .expect("strict cookie should be excluded");

    assert_eq!(
        strict.access_result.status.exclusion_reasons,
        vec![CookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.access_result.status.warning_reasons,
        vec![CookieWarningReason::SameSiteContextDowngradedByRedirect]
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .schemeful_context
            .downgrade_type,
        Some(SameSiteContextDowngradeType::StrictToCross)
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .context
            .downgrade_type,
        None
    );
    assert_eq!(
        strict.access_result.same_site_context.schemeful_context,
        SameSiteRequestContext::CrossSite
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .schemeful_context
            .http_method,
        SameSiteContextHttpMethod::Get
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .schemeful_context
            .redirect_type,
        SameSiteContextRedirectType::NoRedirect
    );
}

#[test]
fn access_query_result_preserves_schemeful_only_redirect_metadata() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("http://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::http(&request_url);
    context.same_site_context = SameSiteContext::new(
        SameSiteRequestContext::SameSiteStrict,
        SameSiteRequestContext::SameSiteLax,
    );
    context.same_site_context_metadata = SameSiteContextMetadata::new(
        SameSiteContextTrackMetadata::none(),
        SameSiteContextTrackMetadata::new(true, Some(SameSiteContextDowngradeType::StrictToLax)),
    );

    let result = store.get_cookie_access_query_result(&context);
    let strict = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "strict")
        .expect("strict cookie should be excluded");

    assert_eq!(
        strict.access_result.status.warning_reasons,
        vec![
            CookieWarningReason::StrictLaxDowngradeStrictSameSite,
            CookieWarningReason::SameSiteContextDowngradedByRedirect,
        ]
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .context
            .downgrade_type,
        None
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .schemeful_context
            .downgrade_type,
        Some(SameSiteContextDowngradeType::StrictToLax)
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .schemeful_context
            .http_method,
        SameSiteContextHttpMethod::Get
    );
}

#[test]
fn access_query_result_records_http_method_and_redirect_type_metadata() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://other.test/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::http_cross_site_top_level_unsafe(&request_url);
    context.redirect_type = SameSiteContextRedirectType::CrossSiteRedirect;
    let result = store.get_cookie_access_query_result(&context);
    let strict = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "strict")
        .expect("strict cookie should be excluded");

    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .context
            .http_method,
        SameSiteContextHttpMethod::Post
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .schemeful_context
            .http_method,
        SameSiteContextHttpMethod::Post
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .context
            .redirect_type,
        SameSiteContextRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        strict
            .access_result
            .same_site_context_metadata
            .schemeful_context
            .redirect_type,
        SameSiteContextRedirectType::CrossSiteRedirect
    );
}

#[test]
fn access_query_result_preserves_browser_site_context_snapshot() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://other.test/foo/bar");
    let site_for_cookies_url = test_utils::url("https://top.example/frame.html");
    let top_frame_origin_url = test_utils::url("https://top.example/root");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::http_cross_site_top_level(&request_url);
    context.browser_context = BrowserSiteContext {
        site_for_cookies_url: Some(site_for_cookies_url.clone()),
        top_frame_origin_url: Some(top_frame_origin_url.clone()),
        storage_access_status: StorageAccessStatus::Granted,
        cookie_partition_key: None,
    };
    let result = store.get_cookie_access_query_result(&context);
    let strict = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "strict")
        .expect("strict cookie should be excluded");

    assert_eq!(
        strict
            .access_result
            .browser_context
            .site_for_cookies_url
            .as_ref(),
        Some(&site_for_cookies_url)
    );
    assert_eq!(
        strict
            .access_result
            .browser_context
            .top_frame_origin_url
            .as_ref(),
        Some(&top_frame_origin_url)
    );
    assert_eq!(
        strict.access_result.browser_context.storage_access_status,
        StorageAccessStatus::Granted
    );
}

#[test]
fn access_query_result_reports_lax_method_unsafe_context_without_lax_inclusion() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://other.test/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "lax=1; Path=/foo; Secure; SameSite=Lax",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "none=1; Path=/foo; Secure; SameSite=None",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_access_query_result(
        &QueryContext::http_cross_site_top_level_unsafe(&request_url),
    );

    let lax = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "lax")
        .expect("lax cookie should be excluded");
    let none = result
        .included_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "none")
        .expect("none cookie should remain included");

    assert_eq!(
        lax.access_result.same_site_context.schemeful_context,
        SameSiteRequestContext::SameSiteLaxMethodUnsafe
    );
    assert_eq!(
        lax.access_result.status.exclusion_reasons,
        vec![CookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        none.access_result.same_site_context.schemeful_context,
        SameSiteRequestContext::SameSiteLaxMethodUnsafe
    );
}

#[test]
fn query_result_projects_from_access_query_result() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "none=1; Path=/foo; Secure; SameSite=None",
        &InsertContext::http(&request_url),
    ));

    let mut context = QueryContext::http(&request_url);
    context.same_site_context = SameSiteContext::new(
        SameSiteRequestContext::SameSiteStrict,
        SameSiteRequestContext::CrossSite,
    );

    let access_result = store.get_cookie_access_query_result(&context);
    let projected_result = store.get_cookie_query_result(&context);

    assert_eq!(access_result.included_cookies.len(), 1);
    assert_eq!(access_result.excluded_cookies.len(), 1);
    assert_eq!(projected_result.included_cookies.len(), 1);
    assert_eq!(projected_result.excluded_cookies.len(), 1);
    assert_eq!(projected_result.included_cookies[0].name(), "none");
    assert_eq!(projected_result.excluded_cookies[0].cookie.name(), "strict");
    assert_eq!(
        projected_result.excluded_cookies[0].reason,
        CookieExclusionReason::SameSiteStrict
    );
}
