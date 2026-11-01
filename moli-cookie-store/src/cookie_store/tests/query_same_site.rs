use super::*;
use time::Duration;

#[test]
fn document_query_filters_same_site_cookies_without_requiring_storage_access() {
    let mut store = CookieStore::default();
    let url = test_utils::url("https://example.com/");
    let mut privileged_write = InsertContext::cdp(&url);
    privileged_write.same_site_context = SameSiteContext::cross_site();
    for line in [
        "strict=1; Secure; SameSite=Strict",
        "lax=1; Secure; SameSite=Lax",
        "none=1; Secure; SameSite=None",
        "unspecified=1; Secure",
        "secret=1; Secure; SameSite=None; HttpOnly",
    ] {
        inserted!(store.insert_response_cookie_str_with_context(line, &privileged_write));
    }

    for storage_access_status in [StorageAccessStatus::None, StorageAccessStatus::Granted] {
        let mut context = QueryContext::document(&url);
        context.browser_context.storage_access_status = storage_access_status;
        let same_site = store.get_cookie_query_result(&context);
        assert_eq!(same_site.included_cookies.len(), 4);
        assert_eq!(same_site.excluded_cookies.len(), 1);
        assert_eq!(
            same_site.excluded_cookies[0].reason,
            CookieExclusionReason::HttpOnly
        );

        context.same_site_context = SameSiteContext::cross_site();
        let cross_site = store.get_cookie_query_result(&context);
        assert_eq!(cross_site.included_cookies.len(), 1);
        assert_eq!(cross_site.included_cookies[0].name(), "none");
        assert_eq!(cross_site.excluded_cookies.len(), 4);
        for (name, reason) in [
            ("strict", CookieExclusionReason::SameSiteStrict),
            ("lax", CookieExclusionReason::SameSiteLax),
            (
                "unspecified",
                CookieExclusionReason::SameSiteUnspecifiedTreatedAsLax,
            ),
            ("secret", CookieExclusionReason::HttpOnly),
        ] {
            let excluded = cross_site
                .excluded_cookies
                .iter()
                .find(|entry| entry.cookie.name() == name)
                .unwrap();
            assert_eq!(excluded.reason, reason);
        }
    }

    let mut cdp = QueryContext::cdp(&url);
    cdp.same_site_context = SameSiteContext::cross_site();
    let privileged = store.get_cookie_query_result(&cdp);
    assert_eq!(privileged.included_cookies.len(), 5);
    assert!(privileged.excluded_cookies.is_empty());
}

#[test]
fn default_same_site_mode_has_the_chromium_two_minute_boundary() {
    let now = OffsetDateTime::from_unix_timestamp(2_000_000_000).unwrap();
    for (created, expected) in [
        (None, CookieEffectiveSameSite::Lax),
        (Some(now), CookieEffectiveSameSite::LaxAllowUnsafe),
        (
            Some(now - Duration::minutes(2)),
            CookieEffectiveSameSite::LaxAllowUnsafe,
        ),
        (
            Some(now - Duration::minutes(2) - Duration::nanoseconds(1)),
            CookieEffectiveSameSite::Lax,
        ),
    ] {
        assert_eq!(
            CookieEffectiveSameSite::for_cookie(None, created, now),
            expected
        );
        assert_eq!(
            CookieEffectiveSameSite::for_cookie(Some(::cookie::SameSite::Lax), created, now),
            CookieEffectiveSameSite::Lax
        );
        assert_eq!(
            CookieEffectiveSameSite::for_cookie(Some(::cookie::SameSite::None), created, now),
            CookieEffectiveSameSite::NoRestriction
        );
    }
}

#[test]
fn default_cookie_age_uses_microseconds_in_memory_and_after_timestamp_roundtrip() {
    let created = OffsetDateTime::from_unix_timestamp(2_000_000_000).unwrap()
        + Duration::nanoseconds(123_456_789);
    let request_url = test_utils::url("https://example.com/");
    let mut cookie = Cookie::parse("sid=value; Secure; Path=/", &request_url).unwrap();
    cookie.set_creation_time(Some(created));
    let canonical = cookie.creation_time().unwrap();
    assert_eq!(canonical.nanosecond(), 123_456_000);
    let restored = crate::cookie_creation_time::from_unix_micros(
        crate::cookie_creation_time::to_unix_micros(canonical).unwrap(),
    )
    .unwrap();
    for stamp in [created, canonical, restored] {
        for (now, expected) in [
            (
                canonical + Duration::minutes(2) + Duration::nanoseconds(999),
                CookieEffectiveSameSite::LaxAllowUnsafe,
            ),
            (
                canonical + Duration::minutes(2) + Duration::microseconds(1),
                CookieEffectiveSameSite::Lax,
            ),
        ] {
            assert_eq!(
                CookieEffectiveSameSite::for_cookie(None, Some(stamp), now),
                expected
            );
        }
    }
    let mut store = CookieStore::default();
    inserted!(store.insert_response_cookie_str("new=value; Secure; Path=/", &request_url));
    assert_eq!(
        store
            .get("example.com", "/", "new")
            .unwrap()
            .creation_time()
            .unwrap()
            .nanosecond()
            % 1_000,
        0
    );
}

#[test]
fn default_cookie_navigation_allowance_is_limited_by_age_and_request_type() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/");
    let now = OffsetDateTime::now_utc();
    for (line, created) in [
        ("recent=1; Path=/; Secure", now),
        ("old=1; Path=/; Secure", now - Duration::minutes(3)),
        ("explicit_lax=1; Path=/; Secure; SameSite=Lax", now),
        ("strict=1; Path=/; Secure; SameSite=Strict", now),
        ("none=1; Path=/; Secure; SameSite=None", now),
    ] {
        let mut cookie = Cookie::parse(line, &request_url).unwrap().into_owned();
        cookie.set_creation_time(Some(created));
        inserted!(store.insert_with_context(cookie, &InsertContext::http(&request_url)));
    }

    let mut non_navigation = QueryContext::http_cross_site_top_level_unsafe(&request_url);
    non_navigation.request_type = HttpRequestType::Subresource;
    for (context, expected) in [
        (QueryContext::http_cross_site(&request_url), vec!["none"]),
        (
            QueryContext::http_cross_site_top_level(&request_url),
            vec!["explicit_lax", "none", "old", "recent"],
        ),
        (
            QueryContext::http_cross_site_top_level_unsafe(&request_url),
            vec!["none", "recent"],
        ),
        (non_navigation, vec!["none"]),
        (
            QueryContext::document(&request_url),
            vec!["explicit_lax", "none", "old", "recent", "strict"],
        ),
    ] {
        let result = store.get_cookie_query_result(&context);
        let mut names = result
            .included_cookies
            .iter()
            .map(|cookie| cookie.name())
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(names, expected);
    }
    let report = store.get_cookie_access_query_result(&QueryContext::http_cross_site(&request_url));
    for name in ["recent", "old"] {
        let excluded = report
            .excluded_cookies
            .iter()
            .find(|entry| entry.cookie.name() == name)
            .unwrap();
        assert_eq!(
            excluded.access_result.status.exclusion_reasons,
            vec![CookieExclusionReason::SameSiteUnspecifiedTreatedAsLax]
        );
        assert_eq!(excluded.cookie.same_site(), None);
    }
}

#[test]
fn overwriting_a_default_cookie_keeps_its_age_but_recreation_starts_a_new_allowance() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/");
    let old_time = (OffsetDateTime::now_utc() - Duration::minutes(3))
        .replace_nanosecond(123_456_000)
        .unwrap();
    let mut cookie = Cookie::parse("sid=old; Path=/; Secure", &request_url)
        .unwrap()
        .into_owned();
    cookie.set_creation_time(Some(old_time));
    inserted!(store.insert_with_context(cookie, &InsertContext::http(&request_url)));
    updated!(store.insert_response_cookie_str("sid=updated; Path=/; Secure", &request_url));
    assert_eq!(
        store
            .get("example.com", "/", "sid")
            .unwrap()
            .creation_time(),
        Some(old_time)
    );
    assert!(store
        .get_cookie_query_result(&QueryContext::http_cross_site_top_level_unsafe(
            &request_url
        ))
        .included_cookies
        .is_empty());

    expired_existing!(
        store.insert_response_cookie_str("sid=; Path=/; Secure; Max-Age=0", &request_url)
    );
    inserted!(store.insert_response_cookie_str("sid=new; Path=/; Secure", &request_url));
    assert!(
        store
            .get("example.com", "/", "sid")
            .unwrap()
            .creation_time()
            .unwrap()
            > old_time
    );
    let result = store.get_cookie_query_result(&QueryContext::http_cross_site_top_level_unsafe(
        &request_url,
    ));
    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(result.included_cookies[0].value(), "new");
}

#[test]
fn cloning_and_rebuilding_a_store_preserve_default_cookie_age() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/");
    let old_time = (OffsetDateTime::now_utc() - Duration::minutes(3))
        .replace_nanosecond(123_456_000)
        .unwrap();
    let mut cookie = Cookie::parse("sid=old; Path=/; Secure", &request_url)
        .unwrap()
        .into_owned();
    cookie.set_creation_time(Some(old_time));
    inserted!(store.insert_with_context(cookie, &InsertContext::http(&request_url)));
    let restored =
        CookieStore::from_cookies(store.iter_any().cloned().map(Ok::<_, ()>), true).unwrap();
    for mut snapshot in [store.clone(), restored] {
        assert_eq!(
            snapshot
                .get("example.com", "/", "sid")
                .unwrap()
                .creation_time(),
            Some(old_time)
        );
        assert!(snapshot
            .get_cookie_query_result(&QueryContext::http_cross_site_top_level_unsafe(
                &request_url
            ))
            .included_cookies
            .is_empty());
    }
}

#[test]
fn restoring_an_unknown_cookie_age_does_not_start_an_unsafe_navigation_allowance() {
    let request_url = test_utils::url("https://example.com/");
    let cookie = Cookie::parse("sid=old; Path=/; Secure", &request_url)
        .unwrap()
        .into_owned();
    let mut store = CookieStore::from_cookies([Ok::<_, ()>(cookie)], true).unwrap();
    updated!(store.insert_response_cookie_str("sid=updated; Path=/; Secure", &request_url));
    assert_eq!(
        store
            .get("example.com", "/", "sid")
            .unwrap()
            .creation_time(),
        None
    );
    assert!(store
        .get_cookie_query_result(&QueryContext::http_cross_site_top_level_unsafe(
            &request_url
        ))
        .included_cookies
        .is_empty());
    let safe_navigation =
        store.get_cookie_query_result(&QueryContext::http_cross_site_top_level(&request_url));
    assert_eq!(safe_navigation.included_cookies.len(), 1);
    assert_eq!(safe_navigation.included_cookies[0].value(), "updated");
}

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
    assert_eq!(result.included_cookies.len(), 1);
    assert!(result
        .included_cookies
        .iter()
        .any(|cookie| cookie.name() == "none"));
    assert_eq!(result.excluded_cookies.len(), 3);
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "unspecified"
            && cookie.reason == CookieExclusionReason::SameSiteUnspecifiedTreatedAsLax
    }));
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
fn document_query_applies_same_site_request_context() {
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
    assert!(result.included_cookies.is_empty());
    assert_eq!(result.excluded_cookies.len(), 1);
    assert_eq!(result.excluded_cookies[0].cookie.name(), "strict");
    assert_eq!(
        result.excluded_cookies[0].reason,
        CookieExclusionReason::SameSiteStrict
    );
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
        CookieEffectiveSameSite::LaxAllowUnsafe
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

#[test]
fn overwrite_reports_use_the_stored_age_for_http_and_chained_cdp_writes() {
    let request_url = test_utils::url("https://example.com/");
    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(123_456_000)
        .unwrap();
    for (created, expected) in [
        (
            Some(now - Duration::minutes(3)),
            CookieEffectiveSameSite::Lax,
        ),
        (
            Some(now - Duration::seconds(30)),
            CookieEffectiveSameSite::LaxAllowUnsafe,
        ),
        (None, CookieEffectiveSameSite::Lax),
    ] {
        for http in [true, false] {
            let mut store = CookieStore::default();
            let mut old = Cookie::parse("sid=old; Secure; Path=/", &request_url)
                .unwrap()
                .into_owned();
            old.set_creation_time(created);
            inserted!(store.insert_with_context(old, &InsertContext::http(&request_url)));
            let report = if http {
                store.set_response_cookie_str_with_access_result(
                    "sid=new; Secure; Path=/",
                    &InsertContext::http(&request_url),
                )
            } else {
                let replacement = Cookie::parse("sid=new; Secure; Path=/", &request_url)
                    .unwrap()
                    .into_owned();
                store.set_with_access_result_chain(
                    replacement,
                    &InsertContext::cdp(&request_url),
                    CookieSetAccessResult {
                        status: CookieSetResult::Accepted(StoreAction::Inserted),
                        rejection_reasons: Vec::new(),
                        warning_reasons: vec![CookieSetWarningReason::PathAttributeIgnored],
                        effective_same_site: Some(CookieEffectiveSameSite::LaxAllowUnsafe),
                    },
                )
            };
            assert_eq!(
                report.status,
                CookieSetResult::Accepted(StoreAction::UpdatedExisting)
            );
            assert_eq!(report.effective_same_site, Some(expected));
            if !http {
                assert_eq!(
                    report.warning_reasons,
                    vec![CookieSetWarningReason::PathAttributeIgnored]
                );
            }
            let stored = store.get("example.com", "/", "sid").unwrap();
            assert_eq!(stored.creation_time(), created);
            assert_eq!(stored.value(), "new");
            let result = store.get_cookie_access_query_result(
                &QueryContext::http_cross_site_top_level_unsafe(&request_url),
            );
            assert_eq!(
                result.included_cookies.len(),
                usize::from(expected == CookieEffectiveSameSite::LaxAllowUnsafe)
            );
            let entry = result
                .included_cookies
                .first()
                .or_else(|| result.excluded_cookies.first())
                .unwrap();
            assert_eq!(entry.access_result.effective_same_site, expected);
        }
    }
}
