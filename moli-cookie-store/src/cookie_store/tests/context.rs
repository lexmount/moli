use super::*;

#[test]
fn query_context_http_starts_with_empty_browser_site_context() {
    let url = test_utils::url("https://example.com/index.html");
    let context = QueryContext::http(&url);

    assert_eq!(context.browser_context, BrowserSiteContext::empty());
}

#[test]
fn query_context_can_carry_browser_site_context_inputs() {
    let request_url = test_utils::url("https://example.com/index.html");
    let site_for_cookies_url = test_utils::url("https://top.example/frame.html");
    let top_frame_origin_url = test_utils::url("https://top.example/root");
    let mut context = QueryContext::http(&request_url);
    context.browser_context = BrowserSiteContext {
        site_for_cookies_url: Some(site_for_cookies_url.clone()),
        top_frame_origin_url: Some(top_frame_origin_url.clone()),
        storage_access_status: StorageAccessStatus::Granted,
        cookie_partition_key: None,
    };

    assert_eq!(
        context.browser_context.site_for_cookies_url.as_ref(),
        Some(&site_for_cookies_url)
    );
    assert_eq!(
        context.browser_context.top_frame_origin_url.as_ref(),
        Some(&top_frame_origin_url)
    );
    assert_eq!(
        context.browser_context.storage_access_status,
        StorageAccessStatus::Granted
    );
}

#[cfg(feature = "public_suffix")]
#[test]
fn public_suffix_list_rejects_non_identical_domain_attribute() {
    let mut store = CookieStore::default().with_suffix_list(make_public_suffix_list());

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "wide=1; Domain=co.uk; Path=/; Secure",
            &InsertContext::http(&test_utils::url("https://foo.co.uk/")),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::PublicSuffix)
    );
}

#[cfg(feature = "public_suffix")]
#[test]
fn shared_public_suffix_list_is_retained_across_store_clones() {
    let suffix_list = std::sync::Arc::new(make_public_suffix_list());
    let store = CookieStore::default().with_shared_suffix_list(std::sync::Arc::clone(&suffix_list));
    let cloned = store.clone();

    let stored = store
        .public_suffix_list
        .as_ref()
        .expect("store should retain the shared PSL");
    let cloned = cloned
        .public_suffix_list
        .as_ref()
        .expect("cloned store should retain the shared PSL");
    assert!(std::sync::Arc::ptr_eq(stored, cloned));
    assert_eq!(
        std::sync::Arc::as_ptr(stored) as *const (),
        std::sync::Arc::as_ptr(&suffix_list) as *const (),
    );
}

#[cfg(feature = "public_suffix")]
#[test]
fn public_suffix_identical_request_host_becomes_host_only() {
    let mut store = CookieStore::default().with_suffix_list(make_public_suffix_list());

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "hostonly=1; Domain=github.io; Path=/; Secure",
            &InsertContext::http(&test_utils::url("https://github.io/")),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let identical = store.get_request_values(&test_utils::url("https://github.io/"));
    let sibling = store.get_request_values(&test_utils::url("https://foo.github.io/"));

    assert_eq!(identical.collect::<Vec<_>>(), vec![("hostonly", "1")]);
    assert!(sibling.collect::<Vec<_>>().is_empty());
}

#[cfg(feature = "public_suffix")]
#[test]
fn dot_prefixed_public_suffix_identical_request_host_becomes_host_only_for_gov_uk() {
    let mut store = CookieStore::default().with_suffix_list(make_public_suffix_list());

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "hostonly=1; Domain=.gov.uk; Path=/; Secure",
            &InsertContext::http(&test_utils::url("https://gov.uk/")),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let identical = store.get_request_values(&test_utils::url("https://gov.uk/"));
    let sibling = store.get_request_values(&test_utils::url("https://nhs.gov.uk/"));

    assert_eq!(identical.collect::<Vec<_>>(), vec![("hostonly", "1")]);
    assert!(sibling.collect::<Vec<_>>().is_empty());
}

#[cfg(feature = "public_suffix")]
#[test]
fn noncanonical_public_suffix_identical_request_host_becomes_host_only_for_gov_uk() {
    let mut store = CookieStore::default().with_suffix_list(make_public_suffix_list());

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "hostonly=1; Domain=GoV.Uk; Path=/; Secure",
            &InsertContext::http(&test_utils::url("https://gov.uk/")),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let identical = store.get_request_values(&test_utils::url("https://gov.uk/"));
    let sibling = store.get_request_values(&test_utils::url("https://nhs.gov.uk/"));

    assert_eq!(identical.collect::<Vec<_>>(), vec![("hostonly", "1")]);
    assert!(sibling.collect::<Vec<_>>().is_empty());
}

#[test]
fn empty_domain_uses_canonicalized_request_host_for_unknown_scheme() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("foo://LOCALhost");

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Path=/",
            &InsertContext::http(&request_url),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let identical = store.get_request_values(&test_utils::url("foo://localhost/"));
    let sibling = store.get_request_values(&test_utils::url("foo://example/"));

    assert_eq!(identical.collect::<Vec<_>>(), vec![("sid", "1")]);
    assert!(sibling.collect::<Vec<_>>().is_empty());
}

#[test]
fn empty_domain_rejects_uncanonicalizable_unknown_scheme_host_like_chromium() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("git://%2eHOST");

    assert!(matches!(
        store.set_response_cookie_str_with_context(
            "sid=1; Path=/",
            &InsertContext::http(&request_url),
        ),
        CookieSetResult::Rejected(_)
    ));

    let identical = store.get_request_values(&test_utils::url("git://host/"));
    assert!(identical.collect::<Vec<_>>().is_empty());
}

#[test]
fn empty_domain_is_accepted_for_file_urls_without_host_like_chromium() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("file:///C:/bar.html");

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Path=/",
            &InsertContext::http(&request_url),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let identical = store.get_request_values(&request_url);

    assert_eq!(identical.collect::<Vec<_>>(), vec![("sid", "1")]);
}

#[test]
fn parent_domain_attribute_is_accepted_for_subdomain_request_host() {
    let mut store = CookieStore::default();

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Domain=globex.com; Path=/",
            &InsertContext::http(&test_utils::url("http://mail.globex.com/")),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let sibling = store.get_request_values(&test_utils::url("http://portal.globex.com/"));
    assert_eq!(sibling.collect::<Vec<_>>(), vec![("sid", "1")]);
}

#[test]
fn subdomain_attribute_is_rejected_for_parent_request_host() {
    let mut store = CookieStore::default();

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Domain=mail.globex.com; Path=/",
            &InsertContext::http(&test_utils::url("http://globex.com/")),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::DomainMismatch)
    );
}

#[test]
fn substring_but_not_subdomain_domain_attribute_is_rejected() {
    let mut store = CookieStore::default();

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Domain=globex.com; Path=/",
            &InsertContext::http(&test_utils::url("http://myglobex.com/")),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::DomainMismatch)
    );
}

#[test]
fn trailing_dot_domain_attribute_is_rejected_when_not_matching_request_host() {
    let mut store = CookieStore::default();

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Domain=.foo.com..; Path=/",
            &InsertContext::http(&test_utils::url("http://foo.com/")),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::DomainMismatch)
    );
    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Domain=.foo.com.; Path=/",
            &InsertContext::http(&test_utils::url("http://foo.com/")),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::DomainMismatch)
    );
}

#[test]
fn identical_ip_domain_attribute_becomes_host_only() {
    let mut store = CookieStore::default();
    let request_url = test_utils::url("http://192.0.2.3/");

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "hostonly=1; Domain=.192.0.2.3; Path=/",
            &InsertContext::http(&request_url),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let identical = store.get_request_values(&request_url);
    let sibling = store.get_request_values(&test_utils::url("http://192.0.2.4/"));

    assert_eq!(identical.collect::<Vec<_>>(), vec![("hostonly", "1")]);
    assert!(sibling.collect::<Vec<_>>().is_empty());
}

#[test]
fn invalid_ip_subdomain_domain_attribute_is_rejected_like_chromium() {
    let mut store = CookieStore::default();

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Domain=192; Path=/",
            &InsertContext::http(&test_utils::url("http://192.0.2.3/")),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::DomainMismatch)
    );
    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Domain=00000000; Path=/",
            &InsertContext::http(&test_utils::url("http://0.0.16.0/0000000")),
        ),
        CookieSetResult::Rejected(CookieSetRejectionReason::DomainMismatch)
    );
}

#[test]
fn unknown_registry_identical_domain_attribute_is_accepted_like_chromium() {
    let mut store = CookieStore::default();

    assert_eq!(
        store.set_response_cookie_str_with_context(
            "sid=1; Domain=qjz9; Path=/",
            &InsertContext::http(&test_utils::url("http://qjz9/")),
        ),
        CookieSetResult::Accepted(StoreAction::Inserted)
    );

    let identical = store.get_request_values(&test_utils::url("http://qjz9/"));
    let subdomain = store.get_request_values(&test_utils::url("http://foo.qjz9/"));

    assert_eq!(identical.collect::<Vec<_>>(), vec![("sid", "1")]);
    assert_eq!(subdomain.collect::<Vec<_>>(), vec![("sid", "1")]);
}
