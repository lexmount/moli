use super::{
    BrowserSiteContext, CookieAccessSemantics, CookieDeleteFilter, CookieEffectiveSameSite,
    CookieExclusionReason, CookieScopeSemantics, CookieSetAccessResult, CookieSetRejectionReason,
    CookieSetResult, CookieSetWarningReason, CookieStore, CookieStoreLimits, CookieWarningReason,
    HttpRequestType, InsertContext, QueryContext, SameSiteContext, SameSiteContextDowngradeType,
    SameSiteContextHttpMethod, SameSiteContextMetadata, SameSiteContextRedirectType,
    SameSiteContextTrackMetadata, SameSiteRequestContext, StorageAccessStatus,
};
use super::{InsertResult, StoreAction};
use crate::cookie::{CanonicalCookieInput, Cookie};
use crate::{CookieError, CookieExpiration, CookiePriority, CookieSourceScheme};
use ::cookie::Cookie as RawCookie;
use time::OffsetDateTime;

use crate::utils::test as test_utils;

macro_rules! inserted {
    ($e: expr) => {
        assert_eq!(Ok(StoreAction::Inserted), $e)
    };
}
macro_rules! updated {
    ($e: expr) => {
        assert_eq!(Ok(StoreAction::UpdatedExisting), $e)
    };
}
macro_rules! expired_existing {
    ($e: expr) => {
        assert_eq!(Ok(StoreAction::ExpiredExisting), $e)
    };
}
macro_rules! domain_mismatch {
    ($e: expr) => {
        assert_eq!(Err(CookieError::DomainMismatch), $e)
    };
}
macro_rules! non_http_scheme {
    ($e: expr) => {
        assert_eq!(Err(CookieError::NonHttpScheme), $e)
    };
}
macro_rules! non_rel_scheme {
    ($e: expr) => {
        assert_eq!(Err(CookieError::NonRelativeScheme), $e)
    };
}
macro_rules! expired_err {
    ($e: expr) => {
        assert_eq!(Err(CookieError::Expired), $e)
    };
}
macro_rules! values_are {
    ($store: expr, $url: expr, $values: expr) => {{
        let mut matched_values = $store
            .matches(&test_utils::url($url))
            .iter()
            .map(|c| &c.value()[..])
            .collect::<Vec<_>>();
        matched_values.sort();

        let mut values: Vec<&str> = $values;
        values.sort();

        assert!(
            matched_values == values,
            "\n{:?}\n!=\n{:?}\n",
            matched_values,
            values
        );
    }};
}

fn add_cookie(
    store: &mut CookieStore,
    cookie: &str,
    url: &str,
    expires: Option<OffsetDateTime>,
    max_age: Option<u64>,
) -> InsertResult {
    store.insert(
        test_utils::make_cookie(cookie, url, expires, max_age),
        &test_utils::url(url),
    )
}

#[cfg(feature = "public_suffix")]
#[derive(Debug)]
struct TestPublicSuffixList;

#[cfg(feature = "public_suffix")]
impl crate::CookiePublicSuffixList for TestPublicSuffixList {
    fn is_public_suffix(&self, domain: &[u8]) -> bool {
        matches!(domain, b"co.uk" | b"gov.uk" | b"github.io")
    }
}

#[cfg(feature = "public_suffix")]
fn make_public_suffix_list() -> TestPublicSuffixList {
    TestPublicSuffixList
}

fn make_match_store() -> CookieStore {
    let mut store = CookieStore::default();
    inserted!(add_cookie(
        &mut store,
        "cookie1=1",
        "http://example.com/foo/bar",
        None,
        Some(60 * 5),
    ));
    inserted!(add_cookie(
        &mut store,
        "cookie2=2; Secure",
        "https://example.com/sec/",
        None,
        Some(60 * 5),
    ));
    inserted!(add_cookie(
        &mut store,
        "cookie3=3; HttpOnly",
        "https://example.com/sec/",
        None,
        Some(60 * 5),
    ));
    inserted!(add_cookie(
        &mut store,
        "cookie4=4; Secure; HttpOnly",
        "https://example.com/sec/",
        None,
        Some(60 * 5),
    ));
    inserted!(add_cookie(
        &mut store,
        "cookie5=5",
        "http://example.com/foo/",
        None,
        Some(60 * 5),
    ));
    inserted!(add_cookie(
        &mut store,
        "cookie6=6",
        "http://example.com/",
        None,
        Some(60 * 5),
    ));
    inserted!(add_cookie(
        &mut store,
        "cookie7=7",
        "http://bar.example.com/foo/",
        None,
        Some(60 * 5),
    ));
    inserted!(add_cookie(
        &mut store,
        "cookie8=8",
        "http://example.org/foo/bar",
        None,
        Some(60 * 5),
    ));
    inserted!(add_cookie(
        &mut store,
        "cookie9=9",
        "http://bar.example.org/foo/bar",
        None,
        Some(60 * 5),
    ));
    store
}

macro_rules! check_matches {
    ($store: expr) => {{
        values_are!($store, "http://unknowndomain.org/foo/bar", vec![]);
        values_are!($store, "http://example.org/foo/bar", vec!["8"]);
        values_are!($store, "http://example.org/bus/bar", vec![]);
        values_are!($store, "http://bar.example.org/foo/bar", vec!["9"]);
        values_are!($store, "http://bar.example.org/bus/bar", vec![]);
        values_are!(
            $store,
            "https://example.com/sec/foo",
            vec!["6", "4", "3", "2"]
        );
        values_are!($store, "http://example.com/sec/foo", vec!["6", "3"]);
        values_are!($store, "ftp://example.com/sec/foo", vec!["6"]);
        values_are!($store, "http://bar.example.com/foo/bar/bus", vec!["7"]);
        values_are!(
            $store,
            "http://example.com/foo/bar/bus",
            vec!["1", "5", "6"]
        );
    }};
}

fn matches_are(store: &CookieStore, url: &str, exp: Vec<&str>) {
    let matches = store
        .matches(&test_utils::url(url))
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>();
    for e in &exp {
        assert!(
            matches.iter().any(|m| &m[..] == *e),
            "{url}: matches missing '{e}'\nmatches: {matches:?}\n    exp: {exp:?}"
        );
    }
    assert!(
        matches.len() == exp.len(),
        "{url}: matches={matches:?} != exp={exp:?}"
    );
}

mod context;
#[path = "tests/query_filter.rs"]
mod query_filter;
#[path = "tests/query_same_site.rs"]
mod query_same_site;
mod store;
mod write;
