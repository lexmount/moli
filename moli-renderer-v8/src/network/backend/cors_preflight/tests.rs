use super::*;
use moli_fetch::{RedirectInfo, RedirectSource, RequestCacheMode};
use moli_storage_key::{MoliStorageKey, OpaqueOriginNonce};
use moli_url::WebOrigin;
use url::Url;

fn request(method: &str) -> Request {
    Request::new_browser_bytes(
        method,
        "https://target.test/resource?version=1",
        None,
        Vec::new(),
        WebOrigin::from_url(&Url::parse("https://origin.test").unwrap()),
    )
    .unwrap()
    .with_credentials_mode(RequestCredentialsMode::Omit)
    .with_network_partition_key(Some("partition-a".to_owned()))
}

fn grant(
    credentials: RequestCredentialsMode,
    methods: &str,
    headers: &str,
    ages: &[&str],
) -> CorsPreflightResult {
    CorsPreflightResult::new(
        credentials,
        Some(methods.split(',').map(str::to_owned).collect()),
        Some(headers.split(',').map(str::to_owned).collect()),
        &ages
            .iter()
            .map(|value| {
                (
                    "Access-Control-Max-Age".to_owned(),
                    value.as_bytes().to_vec(),
                )
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn cache_is_partitioned_by_initiator_origin_and_full_target_url() {
    let now = Instant::now();
    let original = request("PUT");
    let mut cache = CorsPreflightCache::default();
    cache.insert(
        &original,
        grant(RequestCredentialsMode::Omit, "PUT", "", &[]),
        now,
    );
    assert!(cache.can_skip(&original, &[], now));
    let mut different_url = original.clone();
    different_url.url.set_query(Some("version=2"));
    let different_origin = original.clone().with_request_origin(WebOrigin::from_url(
        &Url::parse("https://other.test").unwrap(),
    ));
    for other in [
        different_url,
        different_origin,
        original
            .clone()
            .with_network_partition_key(Some("partition-b".to_owned())),
        original.clone().with_network_partition_key(None),
    ] {
        assert!(!cache.can_skip(&other, &[], now));
        assert!(cache.can_skip(&original, &[], now));
    }
}

#[test]
fn cached_permissions_preserve_case_and_do_not_upgrade_credential_grants() {
    let now = Instant::now();
    let original = request("patcH");
    for (credentials, method, headers, include, expected) in [
        (
            RequestCredentialsMode::Omit,
            "patcH",
            vec![("X-Test", "different")],
            false,
            true,
        ),
        (
            RequestCredentialsMode::Omit,
            "PATCH",
            vec![("x-test", "1")],
            false,
            false,
        ),
        (
            RequestCredentialsMode::Omit,
            "patcH",
            vec![("x-extra", "1")],
            false,
            false,
        ),
        (
            RequestCredentialsMode::Omit,
            "patcH",
            vec![("x-test", "1")],
            true,
            false,
        ),
        (
            RequestCredentialsMode::Include,
            "patcH",
            vec![("x-test", "1")],
            false,
            true,
        ),
        (
            RequestCredentialsMode::Include,
            "patcH",
            vec![("x-test", "1")],
            true,
            true,
        ),
    ] {
        let mut cache = CorsPreflightCache::default();
        cache.insert(&original, grant(credentials, "patcH", "x-test", &[]), now);
        let mut next = original.clone();
        next.method = method.to_owned();
        next.credentials_mode = if include {
            RequestCredentialsMode::Include
        } else {
            RequestCredentialsMode::Omit
        };
        let headers = headers
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(cache.can_skip(&next, &headers, now), expected);
        if !expected {
            // An insufficient grant must not survive a failed replacement
            // preflight and become usable again with the old method/headers.
            assert!(!cache.can_skip(&original, &[("x-test".into(), "1".into())], now));
        }
    }
}

#[test]
fn cached_wildcards_never_authorize_authorization_or_gain_wildcard_semantics() {
    let now = Instant::now();
    for (credentials, headers, expected) in [
        (
            RequestCredentialsMode::Omit,
            vec![("x-arbitrary", "1")],
            true,
        ),
        (
            RequestCredentialsMode::Omit,
            vec![("Authorization", "secret")],
            false,
        ),
        (
            RequestCredentialsMode::Include,
            vec![("x-arbitrary", "1")],
            false,
        ),
    ] {
        let mut cache = CorsPreflightCache::default();
        let original = request("PUT");
        cache.insert(&original, grant(credentials, "PUT,*", "*", &[]), now);
        let headers = headers
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(cache.can_skip(&original, &headers, now), expected);
    }
    // A credentialed grant's literal wildcard method also stays literal when
    // a later request omits credentials.
    let mut cache = CorsPreflightCache::default();
    let original = request("PUT");
    cache.insert(
        &original,
        grant(RequestCredentialsMode::Include, "PUT,*", "", &[]),
        now,
    );
    assert!(!cache.can_skip(&request("DELETE"), &[], now));
}

#[test]
fn expiry_uses_the_default_clamp_and_zero_age_without_extending_old_grants() {
    let now = Instant::now();
    let original = request("PUT");
    for (ages, seconds) in [
        (vec![], 5),
        (vec!["invalid"], 5),
        (vec!["0"], 0),
        (vec!["-1"], 0),
        (vec!["7"], 7),
        (vec!["12000"], 7200),
        (vec!["3", "9"], 5),
        (vec!["9223372036854775808"], 5),
    ] {
        let mut cache = CorsPreflightCache::default();
        cache.insert(
            &original,
            grant(RequestCredentialsMode::Omit, "PUT", "", &["100"]),
            now,
        );
        cache.insert(
            &original,
            grant(RequestCredentialsMode::Omit, "PUT", "", &ages),
            now,
        );
        if seconds > 0 {
            assert!(cache.can_skip(
                &original,
                &[],
                now + Duration::from_secs(seconds) - Duration::from_nanos(1)
            ));
        }
        assert!(!cache.can_skip(&original, &[], now + Duration::from_secs(seconds)));
    }
}

#[test]
fn cache_modes_bypass_lookup_and_no_store_does_not_record_a_grant() {
    let now = Instant::now();
    let original = request("PUT");
    for mode in [
        RequestCacheMode::Default,
        RequestCacheMode::Validate,
        RequestCacheMode::Bypass,
        RequestCacheMode::NoStore,
    ] {
        let mut cache = CorsPreflightCache::default();
        let controlled = original.clone().with_cache_mode(mode);
        cache.insert(
            &controlled,
            grant(RequestCredentialsMode::Omit, "PUT", "", &[]),
            now,
        );
        assert_eq!(
            cache.can_skip(&controlled, &[], now),
            mode == RequestCacheMode::Default
        );
        assert_eq!(
            cache.can_skip(&original, &[], now),
            mode != RequestCacheMode::NoStore
        );
        cache.clear();
        assert!(!cache.can_skip(&original, &[], now));
    }
}

#[test]
fn opaque_grants_require_and_preserve_the_existing_storage_key_nonce() {
    let now = Instant::now();
    let key = |nonce| {
        MoliStorageKey::first_party_from_url(
            &Url::parse("data:,opaque").unwrap(),
            Some(OpaqueOriginNonce::new(nonce)),
        )
        .serialized_storage_key()
    };
    let original = request("PUT")
        .with_request_origin(WebOrigin::Opaque)
        .with_network_partition_key(Some(key(1)));
    let mut cache = CorsPreflightCache::default();
    cache.insert(
        &original,
        grant(RequestCredentialsMode::Omit, "PUT", "", &[]),
        now,
    );
    assert!(cache.can_skip(&original, &[], now));
    let other = original.clone().with_network_partition_key(Some(key(2)));
    assert!(!cache.can_skip(&other, &[], now));
    for partition in [None, Some("null".to_owned())] {
        let unidentified = original.clone().with_network_partition_key(partition);
        cache.insert(
            &unidentified,
            grant(RequestCredentialsMode::Omit, "PUT", "", &[]),
            now,
        );
        assert!(!cache.can_skip(&unidentified, &[], now));
    }
    assert!(cache.can_skip(&original, &[], now));
}

#[test]
fn redirect_tainted_origin_does_not_reuse_an_untainted_grant() {
    let now = Instant::now();
    let original = request("PUT");
    let redirected = original.clone().with_redirect_chain(vec![RedirectInfo {
        source: RedirectSource::Network,
        from_url: Url::parse("https://redirect.test/start").unwrap(),
        to_url: original.url.clone(),
        status: 307,
        headers: Vec::new(),
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }]);
    assert_eq!(redirected.serialized_origin(), "null");
    let mut cache = CorsPreflightCache::default();
    cache.insert(
        &original,
        grant(RequestCredentialsMode::Omit, "PUT", "", &[]),
        now,
    );
    assert!(!cache.can_skip(&redirected, &[], now));
    assert!(cache.can_skip(&original, &[], now));
}

#[test]
fn cache_is_bounded_and_does_not_retain_long_url_keys() {
    let now = Instant::now();
    let mut cache = CorsPreflightCache::default();
    for index in 0..MAX_ENTRIES + 1 {
        let mut next = request("PUT");
        next.url.set_query(Some(&index.to_string()));
        cache.insert(
            &next,
            grant(RequestCredentialsMode::Omit, "PUT", "", &[]),
            now,
        );
    }
    assert_eq!(cache.entries.len(), MAX_ENTRIES);
    let mut long = request("PUT");
    long.url.set_query(Some(&"x".repeat(MAX_URL_LENGTH)));
    cache.insert(
        &long,
        grant(RequestCredentialsMode::Omit, "PUT", "", &[]),
        now,
    );
    assert!(!cache.can_skip(&long, &[], now));
    assert_eq!(cache.entries.len(), MAX_ENTRIES);
}
