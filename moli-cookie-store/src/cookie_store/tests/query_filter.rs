use super::*;

#[test]
fn delete_matching_removes_matching_domain_cookie_but_not_host_only_cookie() {
    let mut store = CookieStore::default();
    let response_url = test_utils::url("https://sub.example.com/app/index.html");

    inserted!(store
        .set_response_cookie_str_with_context(
            "shared=1; Domain=example.com; Path=/app; Secure",
            &InsertContext::http(&response_url),
        )
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "host=1; Path=/app; Secure",
            &InsertContext::http(&response_url),
        )
        .into_insert_result());

    let removed = store.delete_matching(&CookieDeleteFilter {
        name: Some("shared"),
        domain: None,
        path: None,
        url_host: Some("deep.sub.example.com"),
        partition_key: None,
    });

    assert_eq!(removed, 1);
    assert!(store.get("example.com", "/app", "shared").is_none());
    assert!(store.get("sub.example.com", "/app", "host").is_some());
}

#[test]
fn delete_matching_url_host_filter_respects_host_only_semantics() {
    let mut store = CookieStore::default();
    let response_url = test_utils::url("https://example.com/app/index.html");

    inserted!(store
        .set_response_cookie_str_with_context(
            "host=1; Path=/app; Secure",
            &InsertContext::http(&response_url),
        )
        .into_insert_result());
    inserted!(store
        .set_response_cookie_str_with_context(
            "shared=1; Domain=example.com; Path=/app; Secure",
            &InsertContext::http(&response_url),
        )
        .into_insert_result());

    let removed = store.delete_matching(&CookieDeleteFilter {
        name: None,
        domain: None,
        path: None,
        url_host: Some("deep.sub.example.com"),
        partition_key: None,
    });

    assert_eq!(removed, 1);
    assert!(store.get("example.com", "/app", "shared").is_none());
    assert!(store.get("example.com", "/app", "host").is_some());
}

#[test]
fn query_result_reports_document_httponly_exclusion() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "http_only=1; Path=/foo; Secure; HttpOnly",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "visible=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_query_result(&QueryContext::document(&request_url));
    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(result.included_cookies[0].name(), "visible");
    assert_eq!(result.excluded_cookies.len(), 1);
    assert_eq!(
        result.excluded_cookies[0].reason,
        CookieExclusionReason::HttpOnly
    );
    assert_eq!(result.excluded_cookies[0].cookie.name(), "http_only");
}

#[test]
fn query_result_can_skip_collecting_excluded_cookies() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "http_only=1; Path=/foo; Secure; HttpOnly",
        &InsertContext::http(&request_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "visible=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_query_result(
        &QueryContext::document(&request_url).with_return_excluded_cookies(false),
    );
    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(result.included_cookies[0].name(), "visible");
    assert!(result.excluded_cookies.is_empty());
}

#[test]
fn query_result_reports_secure_and_path_exclusions() {
    let mut store = CookieStore::default();
    let secure_url = test_utils::url("https://example.com/foo/bar");
    let insecure_url = test_utils::url("http://example.com/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "secure=1; Path=/; Secure",
        &InsertContext::http(&secure_url),
    ));
    inserted!(store.insert_response_cookie_str_with_context(
        "scoped=1; Path=/foo",
        &InsertContext::http(&secure_url),
    ));

    let result = store.get_cookie_query_result(&QueryContext::http(&insecure_url));
    assert!(result.included_cookies.is_empty());
    assert_eq!(result.excluded_cookies.len(), 2);
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "secure" && cookie.reason == CookieExclusionReason::SecureOnly
    }));
    assert!(result.excluded_cookies.iter().any(|cookie| {
        cookie.cookie.name() == "scoped" && cookie.reason == CookieExclusionReason::PathMismatch
    }));
}

#[test]
fn query_result_is_agnostic_to_source_port() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com:8443/foo/bar");
    let other_port_url = test_utils::url("https://example.com:9443/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));

    // RFC 6265 matching is port-agnostic: a cookie set on one port of a host
    // is visible on every other port of the same host.
    let result = store.get_cookie_query_result(&QueryContext::http(&other_port_url));
    assert!(result.excluded_cookies.is_empty());
    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(result.included_cookies[0].name(), "sid");
}

#[test]
fn host_only_cookie_is_shared_across_ports_of_the_same_host() {
    let mut store = CookieStore::default();
    let set_url = test_utils::url("http://127.0.0.1:8771/");
    let other_port_url = test_utils::url("http://127.0.0.1:8768/check");

    inserted!(store
        .insert_response_cookie_str_with_context("a=1; Path=/", &InsertContext::http(&set_url),));

    let result = store.get_cookie_query_result(&QueryContext::http(&other_port_url));
    assert!(result.excluded_cookies.is_empty());
    assert_eq!(result.included_cookies.len(), 1);
    assert_eq!(result.included_cookies[0].name(), "a");
    assert_eq!(result.included_cookies[0].value(), "1");
}

#[test]
fn query_result_reports_source_scheme_mismatch() {
    let mut store = CookieStore::default();
    let secure_url = test_utils::url("https://example.com:8443/foo/bar");
    let insecure_url = test_utils::url("http://example.com:8443/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=1; Path=/foo",
        &InsertContext::http(&secure_url),
    ));

    let result = store.get_cookie_query_result(&QueryContext::http(&insecure_url));
    assert!(result.included_cookies.is_empty());
    assert_eq!(result.excluded_cookies.len(), 1);
    assert_eq!(result.excluded_cookies[0].cookie.name(), "sid");
    assert_eq!(
        result.excluded_cookies[0].reason,
        CookieExclusionReason::SchemeMismatch
    );
}

#[test]
fn access_query_result_accumulates_multiple_exclusion_reasons() {
    let mut store = CookieStore::default();
    let response_url = test_utils::url("https://example.com:8443/foo/bar");
    let request_url = test_utils::url("http://example.com:9443/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "strict=1; Path=/foo; Secure; SameSite=Strict",
        &InsertContext::http(&response_url),
    ));

    let result = store.get_cookie_access_query_result(&QueryContext::http_cross_site(&request_url));
    let strict = result
        .excluded_cookies
        .iter()
        .find(|entry| entry.cookie.name() == "strict")
        .expect("strict cookie should be excluded");

    assert_eq!(
        strict.access_result.status.exclusion_reasons,
        vec![
            CookieExclusionReason::PathMismatch,
            CookieExclusionReason::SecureOnly,
            CookieExclusionReason::SchemeMismatch,
            CookieExclusionReason::SameSiteStrict,
        ]
    );
}

#[test]
fn query_result_reports_host_only_domain_exclusion() {
    let mut store = CookieStore::default();
    let host_only_url = test_utils::url("https://example.com/");
    let subdomain_url = test_utils::url("https://sub.example.com/");

    inserted!(store.insert_response_cookie_str_with_context(
        "hostonly=1; Path=/; Secure",
        &InsertContext::http(&host_only_url),
    ));

    let result = store.get_cookie_query_result(&QueryContext::http(&subdomain_url));
    assert!(result.included_cookies.is_empty());
    assert_eq!(result.excluded_cookies.len(), 1);
    assert_eq!(result.excluded_cookies[0].cookie.name(), "hostonly");
    assert_eq!(
        result.excluded_cookies[0].reason,
        CookieExclusionReason::DomainMismatch
    );
}

#[test]
fn query_result_reports_expired_cookie() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("https://example.com/foo/bar");

    inserted!(store.insert_response_cookie_str_with_context(
        "sid=1; Path=/foo; Secure",
        &InsertContext::http(&request_url),
    ));
    expired_existing!(store.insert_response_cookie_str_with_context(
        "sid=gone; Path=/foo; Secure; Max-Age=0",
        &InsertContext::http(&request_url),
    ));

    let result = store.get_cookie_query_result(&QueryContext::http(&request_url));
    assert!(result.included_cookies.is_empty());
    assert_eq!(result.excluded_cookies.len(), 1);
    assert_eq!(result.excluded_cookies[0].cookie.name(), "sid");
    assert_eq!(
        result.excluded_cookies[0].reason,
        CookieExclusionReason::Expired
    );
}
