use super::*;
use crate::CookiePartitionKey;

#[test]
fn parse() {
    let mut store = CookieStore::default();
    inserted!(store.parse(
        "cookie1=value1",
        &test_utils::url("http://example.com/foo/bar"),
    ));
    non_rel_scheme!(store.parse("cookie1=value1", &test_utils::url("data:nonrelativescheme"),));
    non_http_scheme!(store.parse(
        "cookie1=value1; HttpOnly",
        &test_utils::url("ftp://example.com/"),
    ));
    expired_existing!(store.parse(
        "cookie1=value1; Max-Age=0",
        &test_utils::url("http://example.com/foo/bar"),
    ));
    expired_err!(store.parse(
        "cookie1=value1; Max-Age=-1",
        &test_utils::url("http://example.com/foo/bar"),
    ));
    inserted!(store.parse(
        "cookie1=value1",
        &test_utils::url("http://example.com/foo/bar"),
    ));
    expired_existing!(store.parse(
        "cookie1=value1; Max-Age=-1",
        &test_utils::url("http://example.com/foo/bar"),
    ));
    domain_mismatch!(store.parse(
        "cookie1=value1; Domain=bar.example.com",
        &test_utils::url("http://example.com/foo/bar"),
    ));
}

#[test]
fn insert_response_cookie_str() {
    let mut store = CookieStore::default();

    inserted!(store.insert_response_cookie_str(
        "cookie1=value1; Path=/foo; Secure",
        &test_utils::url("https://example.com/foo/bar"),
    ));

    assert!(store.get("example.com", "/foo", "cookie1").is_some());
}

#[test]
fn set_response_cookie_str_with_context_reports_rejection_reason() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "__Secure-a=1; Path=/foo",
            &InsertContext::http(&request_url),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::PrefixViolation)
    );
}

#[test]
fn set_response_cookie_str_with_access_result_reports_sanitized_attribute_warnings() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/app/index.html");
    let oversized_domain = format!("{}.com", "a".repeat(1021));
    let invalid_path = "/\u{7f}invalid";

    let result = store.set_response_cookie_str_with_access_result(
        &format!("sid=1; Domain={oversized_domain}; Path={invalid_path}; Secure"),
        &InsertContext::http(&request_url),
    );

    assert!(result.is_accepted());
    assert_eq!(
        result.warning_reasons,
        vec![
            CookieSetWarningReason::DomainAttributeIgnored,
            CookieSetWarningReason::PathAttributeIgnored,
        ]
    );
    assert_eq!(
        result.effective_same_site,
        Some(CookieEffectiveSameSite::LaxAllowUnsafe)
    );

    let cookie = store
        .get("example.com", "/app", "sid")
        .expect("cookie should have fallen back to host-only/default-path");
    assert!(matches!(cookie.domain, crate::CookieDomain::HostOnly(_)));
    assert_eq!(cookie.path.as_ref(), "/app");
}

#[test]
fn set_with_access_result_warns_when_secure_cookie_is_granted_on_localhost_http() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("http://localhost/app/index.html");
    let cookie =
        test_utils::make_cookie("sid=1; Path=/app; Secure", request_url.as_str(), None, None)
            .into_owned();

    let result = store.set_with_access_result(cookie, &InsertContext::http(&request_url));

    assert!(result.is_accepted());
    assert_eq!(
        result.warning_reasons,
        vec![CookieSetWarningReason::SecureAccessGrantedNonCryptographic]
    );
    assert_eq!(
        result.effective_same_site,
        Some(CookieEffectiveSameSite::LaxAllowUnsafe)
    );
}

#[test]
fn set_with_access_result_reports_effective_same_site_for_accepted_cookie() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/account/index.html");
    let cookie = Cookie::parse(
        "sid=1; Path=/account; Secure; SameSite=Strict",
        &request_url,
    )
    .unwrap()
    .into_owned();

    let result = store.set_with_access_result(cookie, &InsertContext::http(&request_url));

    assert_eq!(
        result.status,
        CookieSetResult::Accepted(StoreAction::Inserted)
    );
    assert_eq!(
        result.effective_same_site,
        Some(CookieEffectiveSameSite::Strict)
    );
}

#[test]
fn set_with_access_result_preserves_effective_same_site_for_rejected_cookie() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/account/index.html");
    let cookie = Cookie::parse("sid=1; Path=/account; SameSite=None", &request_url)
        .unwrap()
        .into_owned();

    let result = store.set_with_access_result(cookie, &InsertContext::http(&request_url));

    assert_eq!(
        result.status,
        CookieSetResult::Rejected(CookieSetRejectionReason::SameSiteNoneRequiresSecure)
    );
    assert_eq!(
        result.effective_same_site,
        Some(CookieEffectiveSameSite::NoRestriction)
    );
}

#[test]
fn set_with_access_result_accumulates_multiple_browser_policy_rejections() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/account/index.html");
    let cookie = Cookie::parse("__Host-a=1; SameSite=None", &request_url)
        .unwrap()
        .into_owned();

    let result = store.set_with_access_result(cookie, &InsertContext::http(&request_url));

    assert_eq!(
        result.rejection_reasons,
        vec![
            CookieSetRejectionReason::SameSiteNoneRequiresSecure,
            CookieSetRejectionReason::PrefixViolation,
        ]
    );
}

#[test]
fn set_with_access_result_accumulates_store_side_rejection_reasons() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/account/index.html");
    inserted!(store.insert_response_cookie_str_with_context(
        "sid=secure; Domain=example.com; Path=/account; Secure",
        &InsertContext::http(&request_url),
    ));
    let cookie = Cookie::parse("sid=1; Domain=example.com; Path=/account", &request_url)
        .unwrap()
        .into_owned();
    let insecure_url = test_utils::url("http://sub.example.com/account");
    let insecure_context = InsertContext::http(&insecure_url);

    let result = store.set_with_access_result(cookie, &insecure_context);

    assert_eq!(
        result.rejection_reasons,
        vec![CookieSetRejectionReason::SecureOverlay]
    );
}

#[test]
fn set_with_access_result_chain_preserves_prior_warnings_and_replaces_status() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/account/index.html");
    let cookie = Cookie::parse("sid=1; Path=/account; Secure", &request_url)
        .unwrap()
        .into_owned();
    let prior = CookieSetAccessResult {
        status: CookieSetResult::Accepted(StoreAction::Inserted),
        rejection_reasons: Vec::new(),
        warning_reasons: vec![CookieSetWarningReason::PathAttributeIgnored],
        effective_same_site: None,
    };

    let result =
        store.set_with_access_result_chain(cookie, &InsertContext::http(&request_url), prior);

    assert!(result.is_accepted());
    assert_eq!(
        result.warning_reasons,
        vec![CookieSetWarningReason::PathAttributeIgnored]
    );
}

#[test]
fn set_with_access_result_chain_short_circuits_existing_rejection() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/account/index.html");
    let cookie = Cookie::parse("sid=1; Path=/account; Secure", &request_url)
        .unwrap()
        .into_owned();
    let prior = CookieSetAccessResult {
        status: CookieSetResult::Rejected(CookieSetRejectionReason::PrefixViolation),
        rejection_reasons: vec![CookieSetRejectionReason::PrefixViolation],
        warning_reasons: vec![CookieSetWarningReason::PathAttributeIgnored],
        effective_same_site: Some(CookieEffectiveSameSite::NoRestriction),
    };

    let result = store.set_with_access_result_chain(
        cookie,
        &InsertContext::http(&request_url),
        prior.clone(),
    );

    assert_eq!(result, prior);
}

#[test]
fn set_with_context_reports_insert_update_and_expire_status() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Path=/foo; Secure",
            &InsertContext::http(&request_url)
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );
    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=2; Path=/foo; Secure",
            &InsertContext::http(&request_url)
        ),
        CookieSetResult::Accepted(StoreAction::UpdatedExisting)
    );
    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=gone; Path=/foo; Secure; Max-Age=0",
            &InsertContext::http(&request_url)
        ),
        CookieSetResult::Accepted(StoreAction::ExpiredExisting)
    );
}

#[test]
fn set_with_context_reports_parse_and_expired_rejections() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    assert_eq!(
        store
            .set_response_cookie_str_with_context("bad cookie", &InsertContext::http(&request_url)),
        CookieSetResult::Rejected(CookieSetRejectionReason::Parse)
    );
    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=gone; Path=/foo; Secure; Max-Age=-1",
            &InsertContext::http(&request_url)
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::Expired)
    );
}

#[test]
fn set_with_context_rejects_oversized_name_value() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");
    let oversized_value = "a".repeat(4096);

    assert_eq!(
        store.set_response_cookie_str_with_context(
            &format!("sid={oversized_value}; Path=/foo; Secure"),
            &InsertContext::http(&request_url),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::NameValueTooLarge)
    );
}

#[test]
fn set_with_context_rejects_partitioned_cookie_without_partition_key() {
    let request_url = test_utils::url("https://example.com/foo/bar");

    for context in [
        InsertContext::http(&request_url),
        InsertContext::document(&request_url),
        InsertContext::cdp(&request_url),
    ] {
        let mut store = CookieStore::default();
        assert_eq!(
            store.set_response_cookie_str_with_context(
                "sid=1; Path=/foo; Secure; Partitioned",
                &context,
            ),
            CookieSetResult::Rejected(CookieSetRejectionReason::PartitionedMissingPartitionKey)
        );
    }
}

#[test]
fn legacy_parse_allows_oversized_name_value_without_browser_policy() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");
    let oversized_value = "a".repeat(4096);

    inserted!(store.parse(
        &format!("sid={oversized_value}; Path=/foo; Secure"),
        &request_url,
    ));

    let cookie = store
        .get("example.com", "/foo", "sid")
        .expect("oversized cookie should still be stored by legacy parse");
    assert_eq!(cookie.value(), oversized_value);
}

#[test]
fn canonical_partitioned_cookie_with_explicit_key_is_stored() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");
    let cookie = Cookie::try_from_canonical_input(
        CanonicalCookieInput {
            name: "sid".into(),
            value: "1".into(),
            domain: "example.com".into(),
            host_only: false,
            path: "/foo".into(),
            secure: true,
            http_only: false,
            same_site: None,
            expires: CookieExpiration::SessionEnd,
            partition_key: Some(CookiePartitionKey::site(
                "https://example.com".into(),
                false,
            )),
            priority: Some(CookiePriority::Medium),
            source_scheme: CookieSourceScheme::Secure,
            source_port: 443,
            creation_time: None,
        },
        &request_url,
    )
    .expect("canonical partitioned cookie should parse");

    inserted!(store.insert(cookie, &request_url));

    let cookie = store
        .get_with_partition_key(
            "example.com",
            "/foo",
            "sid",
            Some(&CookiePartitionKey::site(
                "https://example.com".into(),
                false,
            )),
        )
        .expect("partitioned cookie should be stored under its explicit key");
    assert_eq!(cookie.partitioned(), Some(true));
}

#[test]
fn set_canonical_cookie_with_context_round_trips_metadata() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    let result = store.set_canonical_cookie_with_context(
        CanonicalCookieInput {
            name: "sid".into(),
            value: "1".into(),
            domain: "example.com".into(),
            host_only: false,
            path: "/foo".into(),
            secure: true,
            http_only: true,
            same_site: Some(cookie::SameSite::Strict),
            expires: CookieExpiration::AtUtc(test_utils::in_days(1)),
            partition_key: None,
            priority: Some(CookiePriority::High),
            source_scheme: CookieSourceScheme::Secure,
            source_port: 443,
            creation_time: None,
        },
        &InsertContext::http(&request_url),
    );

    assert_eq!(result, CookieSetResult::Accepted(StoreAction::Inserted));
    let cookie = store
        .get("example.com", "/foo", "sid")
        .expect("cookie should exist");
    assert_eq!(cookie.value(), "1");
    assert_eq!(cookie.priority(), Some(CookiePriority::High));
    assert_eq!(cookie.source_scheme(), CookieSourceScheme::Secure);
    assert_eq!(cookie.source_port(), 443);
}

#[test]
fn canonical_host_only_input_preserves_explicit_domain() {
    let request_url = test_utils::url("https://request.example/foo/bar");

    let cookie = Cookie::try_from_canonical_input(
        CanonicalCookieInput {
            name: "sid".into(),
            value: "1".into(),
            domain: "restored.example".into(),
            host_only: true,
            path: "/foo".into(),
            secure: true,
            http_only: false,
            same_site: None,
            expires: CookieExpiration::SessionEnd,
            partition_key: None,
            priority: Some(CookiePriority::Medium),
            source_scheme: CookieSourceScheme::Secure,
            source_port: 443,
            creation_time: None,
        },
        &request_url,
    )
    .expect("host-only canonical input should parse");

    assert!(matches!(
        cookie.domain,
        crate::CookieDomain::HostOnly(ref domain) if domain == "restored.example"
    ));
}

#[test]
fn set_canonical_cookie_with_context_accepts_partitioned_cookie() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    let result = store.set_canonical_cookie_with_context(
        CanonicalCookieInput {
            name: "sid".into(),
            value: "1".into(),
            domain: "example.com".into(),
            host_only: false,
            path: "/foo".into(),
            secure: true,
            http_only: false,
            same_site: None,
            expires: CookieExpiration::SessionEnd,
            partition_key: Some(CookiePartitionKey::site(
                "https://example.com".into(),
                false,
            )),
            priority: Some(CookiePriority::Medium),
            source_scheme: CookieSourceScheme::Secure,
            source_port: 443,
            creation_time: None,
        },
        &InsertContext::http(&request_url),
    );

    assert_eq!(result, CookieSetResult::Accepted(StoreAction::Inserted));
}

#[test]
fn partitioned_cookies_coexist_and_query_by_exact_partition_key() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://cdn.example/foo/bar");
    let key_a = CookiePartitionKey::site("https://top-a.example".into(), true);
    let key_b = CookiePartitionKey::site("https://top-b.example".into(), true);

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=global; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));
    for (key, value) in [(&key_a, "a"), (&key_b, "b")] {
        let mut context = InsertContext::http(&request_url);
        context.browser_context.cookie_partition_key = Some(key.clone());
        inserted!(store.insert_response_cookie_str_with_context(
            &format!("sid={value}; Path=/foo; Secure; Partitioned"),
            &context,
        ));
    }

    assert_eq!(store.iter_unexpired().count(), 3);
    assert_eq!(
        store
            .get_with_partition_key("cdn.example", "/foo", "sid", Some(&key_a))
            .map(|cookie| cookie.value()),
        Some("a")
    );
    assert_eq!(
        store
            .get_with_partition_key("cdn.example", "/foo", "sid", Some(&key_b))
            .map(|cookie| cookie.value()),
        Some("b")
    );

    let mut query = QueryContext::http(&request_url);
    query.browser_context.cookie_partition_key = Some(key_a.clone());
    let result = store.get_ordered_cookie_access_query_result(&query);
    assert_eq!(
        result
            .included_cookies
            .iter()
            .map(|entry| entry.cookie.value())
            .collect::<Vec<_>>(),
        vec!["global", "a"]
    );
    let excluded_b = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.value() == "b")
        .expect("the other partition should be considered but excluded");
    assert!(excluded_b
        .access_result
        .status
        .exclusion_reasons
        .contains(&CookieExclusionReason::PartitionKeyMismatch));

    assert_eq!(
        store.delete_matching(&CookieDeleteFilter {
            name: Some("sid"),
            domain: Some("cdn.example"),
            serialized_domain: None,
            path: Some("/foo"),
            url_host: None,
            partition: CookiePartitionMatch::Exact(&key_a),
        }),
        1
    );
    assert!(store
        .get_with_partition_key("cdn.example", "/foo", "sid", Some(&key_a))
        .is_none());
    assert!(store
        .get_with_partition_key("cdn.example", "/foo", "sid", Some(&key_b))
        .is_some());
    assert!(store.get("cdn.example", "/foo", "sid").is_some());

    assert_eq!(
        store.delete_matching(&CookieDeleteFilter {
            name: Some("sid"),
            partition: CookiePartitionMatch::Unpartitioned,
            ..Default::default()
        }),
        1
    );
    assert!(store.get("cdn.example", "/foo", "sid").is_none());
    assert!(store
        .get_with_partition_key("cdn.example", "/foo", "sid", Some(&key_b))
        .is_some());

    assert_eq!(store.delete_matching(&CookieDeleteFilter::default()), 1);
    assert_eq!(store.iter_unexpired().count(), 0);
}

#[test]
fn partitioned_cookie_requires_secure_even_with_partition_key() {
    let request_url = test_utils::url("https://cdn.example/");
    let mut context = InsertContext::http(&request_url);
    context.browser_context.cookie_partition_key =
        Some(CookiePartitionKey::site("https://top.example".into(), true));
    let mut store = CookieStore::default();

    assert_eq!(
        store.set_response_cookie_str_with_context("sid=1; Path=/; Partitioned", &context),
        CookieSetResult::Rejected(CookieSetRejectionReason::PartitionedRequiresSecure)
    );
}

#[test]
fn set_response_cookie_str_with_context_ignores_oversized_path_attribute() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/app/index.html");
    let oversized_path = format!("/{}", "a".repeat(1024));

    let result = store.set_response_cookie_str_with_access_result(
        &format!("sid=1; Path={oversized_path}; Secure"),
        &InsertContext::http(&request_url),
    );

    assert!(result.is_accepted());
    assert_eq!(
        result.warning_reasons,
        vec![CookieSetWarningReason::PathAttributeIgnored]
    );
    assert!(store.get("example.com", "/app", "sid").is_some());
}

#[test]
fn set_response_cookie_str_with_context_ignores_oversized_domain_attribute() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/app/index.html");
    let oversized_domain = format!("{}.com", "a".repeat(1021));

    let result = store.set_response_cookie_str_with_access_result(
        &format!("sid=1; Domain={oversized_domain}; Secure"),
        &InsertContext::http(&request_url),
    );

    assert!(result.is_accepted());
    assert_eq!(
        result.warning_reasons,
        vec![CookieSetWarningReason::DomainAttributeIgnored]
    );
    let cookie = store
        .get("example.com", "/app", "sid")
        .expect("cookie should exist");
    assert!(matches!(cookie.domain, crate::CookieDomain::HostOnly(_)));
}

#[test]
fn set_response_cookie_str_with_context_ignores_invalid_path_attribute_octets() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/app/index.html");

    let result = store.set_response_cookie_str_with_access_result(
        "sid=1; Path=/\u{7f}invalid; Secure",
        &InsertContext::http(&request_url),
    );

    assert!(result.is_accepted());
    assert_eq!(
        result.warning_reasons,
        vec![CookieSetWarningReason::PathAttributeIgnored]
    );
    assert!(store.get("example.com", "/app", "sid").is_some());
}

#[test]
fn insert_response_cookie_str_records_priority_and_source_metadata() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/app/index.html");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=1; Path=/app; Secure; Priority=High",
        &InsertContext::http(&request_url),
    ));

    let cookie = store
        .get("example.com", "/app", "sid")
        .expect("cookie should exist");
    assert_eq!(cookie.priority(), Some(CookiePriority::High));
    assert_eq!(cookie.source_scheme(), CookieSourceScheme::Secure);
    assert_eq!(cookie.source_port(), 443);
}

#[test]
fn localhost_http_secure_cookie_records_nonsecure_source_scheme() {
    let mut store = CookieStore::default();
    let write_url = test_utils::url("http://localhost:8443/app/index.html");
    let read_url = test_utils::url("https://localhost:8443/app/panel");

    let result = store.set_response_cookie_str_with_access_result(
        "sid=1; Path=/app; Secure",
        &InsertContext::http(&write_url),
    );
    assert!(result.is_accepted());

    let cookie = store
        .get("localhost", "/app", "sid")
        .expect("cookie should exist");
    assert_eq!(cookie.source_scheme(), CookieSourceScheme::NonSecure);

    let query = store.get_cookie_query_result(&QueryContext::http(&read_url));
    assert!(query.included_cookies.is_empty());
    assert_eq!(query.excluded_cookies.len(), 1);
    assert_eq!(
        query.excluded_cookies[0].reason,
        CookieExclusionReason::SchemeMismatch
    );
}

#[test]
fn wss_secure_cookie_write_does_not_warn_as_non_cryptographic() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("wss://example.com/app/index.html");

    let result = store.set_response_cookie_str_with_access_result(
        "sid=1; Path=/app; Secure",
        &InsertContext::http(&request_url),
    );

    assert!(result.is_accepted());
    assert!(result.warning_reasons.is_empty());
}

#[test]
fn insert_with_context_rejects_httponly_from_document_source() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");
    let cookie = Cookie::parse("sid=1; Path=/foo; HttpOnly", &request_url)
        .unwrap()
        .into_owned();

    assert_eq!(
        store.insert_with_context(cookie, &InsertContext::document(&request_url)),
        Err(CookieError::NonHttpScheme)
    );
}

#[test]
fn query_context_hides_httponly_for_document_source_and_touches_access_time() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "http_only=1; Path=/foo; Secure; HttpOnly",
        &InsertContext::http(&request_url),
    ));
    let before = store
        .get("example.com", "/foo", "http_only")
        .expect("cookie should exist")
        .last_access_index();

    let result = store.get_cookies_with_context(&QueryContext::document(&request_url));

    assert!(result.is_empty());
    let after = store
        .get("example.com", "/foo", "http_only")
        .expect("cookie should still exist")
        .last_access_index();
    assert_eq!(after, before);
}

#[test]
fn query_context_can_disable_access_time_updates_for_observation() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));
    let before = store
        .get("example.com", "/foo", "sid")
        .expect("cookie should exist")
        .last_access_index();

    let _ = store.get_cookie_access_query_result(
        &QueryContext::http(&request_url).with_update_access_time(false),
    );

    let after = store
        .get("example.com", "/foo", "sid")
        .expect("cookie should still exist")
        .last_access_index();
    assert_eq!(after, before);
}

#[test]
fn query_context_allows_httponly_for_http_source() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "http_only=1; Path=/foo; Secure; HttpOnly",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookies_with_context(&QueryContext::http(&request_url));

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name(), "http_only");
}

#[test]
fn query_context_allows_secure_and_httponly_cookies_for_wss_requests() {
    let mut store = CookieStore::default();
    let response_url = test_utils::url("https://example.com/socket");
    let request_url = test_utils::url("wss://example.com/socket");

    inserted!(store.insert_response_cookie_str_with_context(
        "http_only=1; Path=/; Secure; HttpOnly",
        &InsertContext::http(&response_url),
    ));

    let result = store.get_cookies_with_context(&QueryContext::http(&request_url));

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name(), "http_only");
}

#[test]
fn ordered_query_result_sorts_by_longest_path_then_creation_time() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "first=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "second=2; Path=/foo/bar; Secure",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "third=3; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));

    let ordered = store.get_ordered_cookies_with_context(&QueryContext::http(&request_url));

    assert_eq!(
        ordered
            .iter()
            .map(|cookie| cookie.name())
            .collect::<Vec<_>>(),
        vec!["second", "first", "third"]
    );
}

#[test]
fn ordered_request_values_with_context_uses_browser_projection_order() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "first=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "second=2; Path=/foo/bar; Secure",
        &InsertContext::http(&request_url),
    ));
    updated!(store.insert_response_cookie_str_with_context(
        "first=3; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));

    assert_eq!(
        store.get_ordered_request_values_with_context(&QueryContext::http(&request_url)),
        vec![
            ("second".to_owned(), "2".to_owned()),
            ("first".to_owned(), "3".to_owned()),
        ]
    );
}

#[test]
fn same_site_none_requires_secure_in_context_insert() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    assert_eq!(
        store.insert_response_cookie_str_with_context(
            "cross=1; Path=/foo; SameSite=None",
            &InsertContext::http(&request_url),
        ),
        Err(CookieError::SameSiteNoneRequiresSecure)
    );

    inserted!(store.insert_response_cookie_str_with_context(
        "cross=1; Path=/foo; SameSite=None; Secure",
        &InsertContext::http(&request_url),
    ));
}

#[test]
fn protected_prefix_rules_are_enforced_in_context_insert() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/account/index.html");

    assert_eq!(
        store.insert_response_cookie_str_with_context(
            "__Secure-a=1; Path=/account",
            &InsertContext::http(&request_url),
        ),
        Err(CookieError::PrefixViolation)
    );
    assert_eq!(
        store.insert_response_cookie_str_with_context(
            "__Host-a=1; Secure",
            &InsertContext::http(&request_url),
        ),
        Err(CookieError::PrefixViolation)
    );
    inserted!(store.insert_response_cookie_str_with_context(
        "__Host-b=1; Secure; Path=/",
        &InsertContext::http(&request_url),
    ));

    let cookie = store
        .get("example.com", "/", "__Host-b")
        .expect("cookie should exist");
    assert_eq!(cookie.value(), "1");
}

#[test]
fn host_prefix_accepts_identical_ip_domain_after_host_only_downgrade() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://192.0.2.3/account/index.html");

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "__Host-ip=1; Domain=.192.0.2.3; Path=/; Secure",
            &InsertContext::http(&request_url),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let cookie = store
        .get("192.0.2.3", "/", "__Host-ip")
        .expect("cookie should exist after host-only downgrade");
    assert!(matches!(cookie.domain, crate::CookieDomain::HostOnly(_)));
}

#[cfg(feature = "public_suffix")]
#[test]
fn host_prefix_accepts_identical_public_suffix_domain_after_host_only_downgrade() {
    let mut store = CookieStore::default().with_suffix_list(make_public_suffix_list());
    let request_url = test_utils::url("https://github.io/account/index.html");

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "__Host-psl=1; Domain=github.io; Path=/; Secure",
            &InsertContext::http(&request_url),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let cookie = store
        .get("github.io", "/", "__Host-psl")
        .expect("cookie should exist after host-only downgrade");
    assert!(matches!(cookie.domain, crate::CookieDomain::HostOnly(_)));
}

#[test]
fn empty_name_cookie_cannot_smuggle_protected_prefix_in_value() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/");
    let raw_cookie = RawCookie::build(("", "__Secure-token"))
        .path("/")
        .secure(true)
        .build();
    let cookie = Cookie::try_from_raw_cookie(&raw_cookie, &request_url)
        .expect("raw cookie should convert")
        .into_owned();

    assert_eq!(
        store.insert_with_context(cookie, &InsertContext::http(&request_url),),
        Err(CookieError::PrefixViolation)
    );
}

#[test]
fn insecure_context_cannot_overlay_existing_secure_cookie() {
    let mut store = CookieStore::default();
    let secure_url = test_utils::url("https://example.com/login");
    let insecure_url = test_utils::url("http://sub.example.com/login");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=secure; Domain=example.com; Path=/login; Secure",
        &InsertContext::http(&secure_url),
    ));

    assert_eq!(
        store.insert_response_cookie_str_with_context(
            "sid=insecure; Domain=example.com; Path=/login",
            &InsertContext::http(&insecure_url),
        ),
        Err(CookieError::SecureOverlay)
    );
}

#[test]
fn cdp_context_can_bypass_secure_overlay_guard() {
    let mut store = CookieStore::default();
    let secure_url = test_utils::url("https://example.com/login");
    let insecure_url = test_utils::url("http://sub.example.com/login");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=secure; Domain=example.com; Path=/login; Secure",
        &InsertContext::http(&secure_url),
    ));
    updated!(store.insert_response_cookie_str_with_context(
        "sid=debug; Domain=example.com; Path=/login",
        &InsertContext::cdp(&insecure_url),
    ));

    let cookie = store
        .get("example.com", "/login", "sid")
        .expect("cookie should exist");
    assert_eq!(cookie.value(), "debug");
    assert!(!cookie.secure().unwrap_or(false));
}

#[test]
fn replacement_preserves_creation_index() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str("cookie1=value1; Path=/foo; Secure", &request_url,));
    let original_creation = store
        .get("example.com", "/foo", "cookie1")
        .expect("cookie should exist")
        .creation_index();

    updated!(store.insert_response_cookie_str("cookie1=value2; Path=/foo; Secure", &request_url,));

    let updated_cookie = store
        .get("example.com", "/foo", "cookie1")
        .expect("cookie should exist after replacement");
    assert_eq!(updated_cookie.value(), "value2");
    assert_eq!(updated_cookie.creation_index(), original_creation);
}

#[test]
fn expired_tombstone_does_not_bypass_total_cookie_limit_for_reinserted_key() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(10, 1));
    let example_url = test_utils::url("https://example.com/foo/bar");
    let other_url = test_utils::url("https://other.example/foo/bar");

    inserted!(store.insert_response_cookie_str("sid=old; Path=/foo; Secure", &example_url,));
    expired_existing!(
        store.insert_response_cookie_str("sid=old; Path=/foo; Secure; Max-Age=0", &example_url,)
    );
    inserted!(store.insert_response_cookie_str("other=1; Path=/foo; Secure", &other_url,));

    inserted!(store.insert_response_cookie_str("sid=new; Path=/foo; Secure", &example_url,));

    assert_eq!(store.iter_unexpired().count(), 1);
    assert_eq!(
        store.get_request_values_with_context(&QueryContext::http(&example_url)),
        vec![("sid".to_owned(), "new".to_owned())]
    );
    assert!(store
        .get_request_values_with_context(&QueryContext::http(&other_url))
        .is_empty());
}

#[test]
fn expired_tombstone_does_not_bypass_per_domain_limit_for_reinserted_key() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(1, 10));
    let example_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str("sid=old; Path=/foo; Secure", &example_url,));
    expired_existing!(
        store.insert_response_cookie_str("sid=old; Path=/foo; Secure; Max-Age=0", &example_url,)
    );
    inserted!(store.insert_response_cookie_str("other=1; Path=/foo; Secure", &example_url,));

    inserted!(store.insert_response_cookie_str("sid=new; Path=/foo; Secure", &example_url,));

    assert_eq!(store.iter_unexpired().count(), 1);
    assert_eq!(
        store.get_request_values_with_context(&QueryContext::http(&example_url)),
        vec![("sid".to_owned(), "new".to_owned())]
    );
    assert!(store.get("example.com", "/foo", "other").is_none());
}

#[test]
fn touch_updates_last_access_index() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str("cookie1=value1; Path=/foo; Secure", &request_url,));
    let before = store
        .get("example.com", "/foo", "cookie1")
        .expect("cookie should exist")
        .last_access_index();

    assert!(store.touch("example.com", "/foo", "cookie1"));

    let after = store
        .get("example.com", "/foo", "cookie1")
        .expect("cookie should still exist")
        .last_access_index();
    assert!(after > before);
}

#[test]
fn per_domain_eviction_prefers_removing_non_secure_cookie() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(5, 100));
    let url = test_utils::url("https://example.com/");

    for index in 1..=4 {
        inserted!(store
            .set_response_cookie_str_with_context(
                &format!("secure{index}=1; Path=/; Secure"),
                &InsertContext::http(&url),
            )
            .into_insert_result());
    }
    inserted!(store
        .set_response_cookie_str_with_context("plain=1; Path=/", &InsertContext::http(&url),)
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "newsecure=1; Path=/; Secure",
            &InsertContext::http(&url),
        )
        .into_insert_result());

    let mut header = store
        .get_request_values_with_context(&QueryContext::http(&url))
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>();
    header.sort();
    assert_eq!(
        header,
        vec![
            "newsecure=1".to_owned(),
            "secure1=1".to_owned(),
            "secure2=1".to_owned(),
            "secure3=1".to_owned(),
            "secure4=1".to_owned(),
        ]
    );
}

#[test]
fn per_domain_eviction_rejects_new_non_secure_cookie_when_all_existing_are_secure() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(5, 100));
    let url = test_utils::url("https://example.com/");

    for index in 1..=5 {
        inserted!(store
            .set_response_cookie_str_with_context(
                &format!("secure{index}=1; Path=/; Secure"),
                &InsertContext::http(&url),
            )
            .into_insert_result());
    }

    assert_eq!(
        store.set_response_cookie_str_with_context("plain=1; Path=/", &InsertContext::http(&url)),
        CookieSetResult::Rejected(CookieSetRejectionReason::StorageFull)
    );
}

#[test]
fn global_eviction_applies_when_total_cookie_limit_is_hit() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(10, 3));

    inserted!(store
        .set_response_cookie_str_with_context(
            "a=1; Path=/; Secure",
            &InsertContext::http(&test_utils::url("https://one.example/")),
        )
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "b=1; Path=/; Secure",
            &InsertContext::http(&test_utils::url("https://two.example/")),
        )
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "c=1; Path=/",
            &InsertContext::http(&test_utils::url("https://three.example/")),
        )
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "d=1; Path=/; Secure",
            &InsertContext::http(&test_utils::url("https://four.example/")),
        )
        .into_insert_result());

    assert_eq!(store.iter_unexpired().count(), 3);
    assert!(store
        .get_request_values_with_context(&QueryContext::http(&test_utils::url(
            "https://three.example/"
        )))
        .is_empty());
    assert_eq!(
        store.get_request_values_with_context(&QueryContext::http(&test_utils::url(
            "https://four.example/"
        ))),
        vec![("d".to_owned(), "1".to_owned())]
    );
}

#[test]
fn partitioned_cookies_do_not_use_global_cookie_quota() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(2, 1));
    let login_url = test_utils::url("https://login.example/");
    inserted!(store.insert_response_cookie_str("A2=login; Path=/; HttpOnly", &login_url));

    let key = CookiePartitionKey::site("https://top.example".into(), true);
    for host in ["one.example", "two.example"] {
        let url = test_utils::url(&format!("https://{host}/"));
        let mut context = InsertContext::http(&url);
        context.browser_context.cookie_partition_key = Some(key.clone());
        inserted!(store.insert_response_cookie_str_with_context(
            "chip=1; Path=/; Secure; Partitioned",
            &context,
        ));
    }

    assert!(store.contains("login.example", "/", "A2"));
    assert_eq!(store.iter_unexpired().count(), 3);
}

#[test]
fn unpartitioned_global_eviction_does_not_remove_partitioned_cookies() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(1, 1));
    let key = CookiePartitionKey::site("https://top.example".into(), true);
    let url = test_utils::url("https://cdn.example/");
    let mut context = InsertContext::http(&url);
    context.browser_context.cookie_partition_key = Some(key.clone());
    inserted!(store
        .insert_response_cookie_str_with_context("chip=1; Path=/; Secure; Partitioned", &context,));

    for host in ["one.example", "two.example"] {
        inserted!(store.insert_response_cookie_str(
            "sid=1; Path=/; Secure",
            &test_utils::url(&format!("https://{host}/")),
        ));
    }

    assert!(store
        .get_with_partition_key("cdn.example", "/", "chip", Some(&key))
        .is_some());
    assert!(!store.contains("one.example", "/", "sid"));
    assert!(store.contains("two.example", "/", "sid"));
    assert_eq!(store.iter_unexpired().count(), 2);
}

#[test]
fn per_domain_cookie_eviction_is_partition_scoped() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(1, 100));
    let url = test_utils::url("https://cdn.example/");
    let keys = [
        None,
        Some(CookiePartitionKey::site(
            "https://top-a.example".into(),
            false,
        )),
        Some(CookiePartitionKey::site(
            "https://top-a.example".into(),
            true,
        )),
        Some(CookiePartitionKey::site(
            "https://top-b.example".into(),
            true,
        )),
        Some(CookiePartitionKey::opaque(1, true)),
        Some(CookiePartitionKey::opaque(2, true)),
    ];
    for key in &keys {
        let mut context = InsertContext::http(&url);
        context.browser_context.cookie_partition_key = key.clone();
        let partitioned = if key.is_some() { "; Partitioned" } else { "" };
        inserted!(store.insert_response_cookie_str_with_context(
            &format!("sid=old; Path=/; Secure{partitioned}"),
            &context,
        ));
    }
    assert_eq!(store.iter_unexpired().count(), keys.len());

    for key in &keys {
        let mut context = InsertContext::http(&url);
        context.browser_context.cookie_partition_key = key.clone();
        let partitioned = if key.is_some() { "; Partitioned" } else { "" };
        updated!(store.insert_response_cookie_str_with_context(
            &format!("sid=updated; Path=/; Secure{partitioned}"),
            &context,
        ));
        assert_eq!(store.iter_unexpired().count(), keys.len());
        inserted!(store.insert_response_cookie_str_with_context(
            &format!("next=1; Path=/; Secure{partitioned}"),
            &context,
        ));
        assert!(store
            .get_with_partition_key("cdn.example", "/", "sid", key.as_ref())
            .is_none());
        assert!(store
            .get_with_partition_key("cdn.example", "/", "next", key.as_ref())
            .is_some());
        assert_eq!(store.iter_unexpired().count(), keys.len());
    }
}

#[test]
fn eviction_prefers_lower_priority_before_higher_priority() {
    let mut store = CookieStore::default().with_limits(CookieStoreLimits::new(3, 100));
    let url = test_utils::url("https://example.com/");

    inserted!(store
        .set_response_cookie_str_with_context(
            "low=1; Path=/; Priority=Low",
            &InsertContext::http(&url),
        )
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "high=1; Path=/; Priority=High",
            &InsertContext::http(&url),
        )
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "medium=1; Path=/; Priority=Medium",
            &InsertContext::http(&url),
        )
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "high2=1; Path=/; Priority=High",
            &InsertContext::http(&url),
        )
        .into_insert_result());

    let mut header = store
        .get_request_values_with_context(&QueryContext::http(&url))
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>();
    header.sort();
    assert_eq!(
        header,
        vec![
            "high2=1".to_owned(),
            "high=1".to_owned(),
            "medium=1".to_owned(),
        ]
    );
}
