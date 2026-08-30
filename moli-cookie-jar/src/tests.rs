use super::test_support::{
    BrowserCookieStore, CookieSiteDataChange, NetworkSameSiteContext,
    NetworkSameSiteContextDowngradeType, NetworkSiteContext,
};
use super::{
    CookiePriority, CookieSource, NetworkCookieRequestContext, NetworkSiteContextMetadata,
    NetworkStorageAccessStatus, StoredCookie, StoredCookieAccessSemantics,
    StoredCookieEffectiveSameSite, StoredCookieExclusionReason, StoredCookiePartitionKey,
    StoredCookieQueryReport, StoredCookieRequestSameSiteContext, StoredCookieSameSite,
    StoredCookieSameSiteContextDowngradeType, StoredCookieSameSiteHttpMethod,
    StoredCookieSameSiteRedirectType, StoredCookieScopeSemantics, StoredCookieSetRejectionReason,
    StoredCookieSetWarningReason, StoredCookieSiteContextBasis, StoredCookieSourceScheme,
    StoredCookieStorageAccessStatus, StoredCookieWarningReason, advance_cookie_request_context,
};
use cookie_store::Cookie as StoreCookie;
use url::Url;

fn parse(url: &str) -> Url {
    Url::parse(url).unwrap()
}

fn site_summary(
    name: &str,
    cookie_count: usize,
    persistent_cookie_count: usize,
) -> super::CookieSiteDataSummary {
    super::CookieSiteDataSummary::new(name.to_owned(), cookie_count, persistent_cookie_count)
}

fn site_change(
    name: &str,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> CookieSiteDataChange {
    CookieSiteDataChange {
        name: name.to_owned(),
        before: before.map(|(cookie_count, persistent_cookie_count)| {
            site_summary(name, cookie_count, persistent_cookie_count)
        }),
        after: after.map(|(cookie_count, persistent_cookie_count)| {
            site_summary(name, cookie_count, persistent_cookie_count)
        }),
    }
}

#[test]
fn network_query_context_projects_browser_site_context_into_core() {
    let request_url = parse("https://example.com/app/panel");
    let site_for_cookies_url = parse("https://top.example/frame.html");
    let top_frame_origin_url = parse("https://top.example/root");
    let request_context = NetworkCookieRequestContext::subresource("GET")
        .with_site_for_cookies_url(&request_url, &site_for_cookies_url)
        .with_top_frame_origin_url(&request_url, &top_frame_origin_url)
        .with_storage_access_status(NetworkStorageAccessStatus::Granted);

    let context = super::jar::network_query_context(&request_url, request_context);

    assert_eq!(
        context
            .browser_context
            .site_for_cookies_url
            .as_ref()
            .map(Url::as_str),
        Some("https://top.example/frame.html")
    );
    assert_eq!(
        context
            .browser_context
            .top_frame_origin_url
            .as_ref()
            .map(Url::as_str),
        Some("https://top.example/root")
    );
    assert_eq!(
        context.browser_context.storage_access_status,
        cookie_store::StorageAccessStatus::Granted
    );
}

fn find_cookie<'a>(
    report: &'a StoredCookieQueryReport,
    name: &str,
) -> Option<&'a super::StoredCookieAccess> {
    report
        .included_cookies
        .iter()
        .chain(report.excluded_cookies.iter())
        .find(|entry| entry.cookie.name == name)
}

#[test]
fn document_cookie_reads_network_cookie_but_hides_httponly() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    store.store_response_headers(
        &url,
        &[
            ("set-cookie".to_owned(), "theme=dark; Path=/app".to_owned()),
            (
                "set-cookie".to_owned(),
                "secret=server; Path=/app; HttpOnly".to_owned(),
            ),
        ],
    );

    assert_eq!(store.document_cookie(&url), "theme=dark");
    assert_eq!(
        store.cookie_header(&parse("https://example.com/app/panel")),
        Some("theme=dark; secret=server".to_owned())
    );
}

#[test]
fn document_cookie_cannot_overwrite_existing_httponly_cookie() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=server; Path=/app; HttpOnly".to_owned(),
        )],
    );
    store.set_document_cookie(&url, "sid=client; Path=/app");

    assert_eq!(store.document_cookie(&url), "");
    assert_eq!(
        store.cookie_header(&parse("https://example.com/app/panel")),
        Some("sid=server".to_owned())
    );
}

#[test]
fn document_cookie_httponly_guard_reads_canonical_core() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=server; Path=/app; HttpOnly".to_owned(),
        )],
    );

    // The wrapper no longer keeps a second cookie mirror. HttpOnly protection must come straight
    // from the canonical core entry.
    store.set_document_cookie(&url, "sid=client; Path=/app");

    assert_eq!(store.document_cookie(&url), "");
    assert_eq!(
        store.cookie_header(&parse("https://example.com/app/panel")),
        Some("sid=server".to_owned())
    );
}

#[test]
fn document_cookie_set_report_surfaces_wrapper_invalid_octet_rejection() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    let report = store.set_document_cookie_with_report(&url, "sid=\u{7f}broken; Path=/app");

    assert_eq!(
        report.status,
        super::StoredCookieSetStatus::Rejected(StoredCookieSetRejectionReason::InvalidOctets)
    );
    assert_eq!(
        report.rejection_reasons,
        vec![StoredCookieSetRejectionReason::InvalidOctets]
    );
    assert!(report.warning_reasons.is_empty());
    assert_eq!(report.effective_same_site, None);
    assert_eq!(store.document_cookie(&url), "");
}

#[test]
fn unknown_scheme_uncanonicalizable_host_is_rejected_like_chromium_cookie_util() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("git://%2eHOST");

    let report = store.store_response_headers_with_reports(
        &request_url,
        &[("set-cookie".to_owned(), "sid=1; Path=/".to_owned())],
    );

    assert_eq!(report.len(), 1);
    assert!(!report[0].is_accepted());
    assert_eq!(store.cookie_header(&parse("git://host/")), None);
}

#[test]
fn file_url_without_host_accepts_host_cookie_like_chromium_cookie_util() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("file:///C:/bar.html");

    let report = store.store_response_headers_with_reports(
        &request_url,
        &[("set-cookie".to_owned(), "sid=1; Path=/".to_owned())],
    );

    assert_eq!(report.len(), 1);
    assert!(report[0].is_accepted());
    assert_eq!(store.cookie_header(&request_url), Some("sid=1".to_owned()));
}

#[test]
fn blob_file_underlying_url_accepts_response_cookie_like_chromium_site_for_cookies() {
    let mut store = BrowserCookieStore::default();
    let nested_url = parse("file:///C:/bar.html");
    let blob_url = parse("blob:file:///C:/bar.html");

    let report = store.store_response_headers_with_reports(
        &nested_url,
        &[("set-cookie".to_owned(), "sid=1; Path=/".to_owned())],
    );

    assert_eq!(report.len(), 1);
    assert!(report[0].is_accepted());
    assert_eq!(store.document_cookie(&blob_url), "sid=1");
}

#[test]
fn store_response_headers_accepts_mixed_case_set_cookie_name() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    let reports = store.store_response_headers_with_reports(
        &url,
        &[("Set-Cookie".to_owned(), "sid=1; Path=/app".to_owned())],
    );

    assert_eq!(reports.len(), 1);
    assert!(reports[0].is_accepted());
    assert_eq!(store.document_cookie(&url), "sid=1");
}

#[test]
fn document_cookie_set_report_accumulates_core_rejection_reasons() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    let report = store.set_document_cookie_with_report(&url, "__Host-sid=1; Path=/; SameSite=None");

    assert_eq!(
        report.status,
        super::StoredCookieSetStatus::Rejected(
            StoredCookieSetRejectionReason::SameSiteNoneRequiresSecure
        )
    );
    assert_eq!(
        report.rejection_reasons,
        vec![
            StoredCookieSetRejectionReason::SameSiteNoneRequiresSecure,
            StoredCookieSetRejectionReason::PrefixViolation,
        ]
    );
    assert_eq!(
        report.effective_same_site,
        Some(super::StoredCookieEffectiveSameSite::NoRestriction)
    );
}

#[test]
fn document_cookie_set_report_accepts_secure_blob_url_like_underlying_origin() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:https://example.org/9115d58c-bcda-ff47-86e5-083e9a2153041");
    let nested_url = parse("https://example.org/resource");

    let report =
        store.set_document_cookie_with_report(&document_url, "sid=1; Path=/; Secure; SameSite=Lax");

    assert!(report.is_accepted());
    assert_eq!(store.document_cookie(&document_url), "sid=1");
    assert_eq!(store.document_cookie(&nested_url), "sid=1");
}

#[test]
fn document_cookie_set_report_accepts_blob_file_url_like_underlying_origin() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:file:///C:/app/index.html");
    let nested_url = parse("file:///C:/app/index.html");

    let report = store.set_document_cookie_with_report(&document_url, "sid=1; Path=/");

    assert!(report.is_accepted());
    assert_eq!(store.document_cookie(&document_url), "sid=1");
    assert_eq!(store.document_cookie(&nested_url), "sid=1");
}

#[test]
fn document_cookie_set_report_accepts_blob_http_url_like_underlying_origin() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:http://example.org/9115d58c-bcda-ff47-86e5-083e9a2153041");
    let nested_url = parse("http://example.org/resource");

    let report = store.set_document_cookie_with_report(&document_url, "sid=1; Path=/");

    assert!(report.is_accepted());
    assert_eq!(store.document_cookie(&document_url), "sid=1");
    assert_eq!(store.document_cookie(&nested_url), "sid=1");
}

#[test]
fn document_cookie_access_report_projects_httponly_exclusion() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    store.store_response_headers(
        &url,
        &[
            ("set-cookie".to_owned(), "theme=dark; Path=/app".to_owned()),
            (
                "set-cookie".to_owned(),
                "secret=server; Path=/app; HttpOnly".to_owned(),
            ),
        ],
    );

    let report = store.document_cookie_access_report(&url);
    let theme = find_cookie(&report, "theme").expect("theme cookie should be present");
    let secret = find_cookie(&report, "secret").expect("secret cookie should be present");

    assert!(
        report
            .included_cookies
            .iter()
            .any(|entry| entry.cookie.name == "theme")
    );
    assert!(
        report
            .excluded_cookies
            .iter()
            .any(|entry| entry.cookie.name == "secret")
    );
    assert_eq!(
        theme.exclusion_reasons,
        Vec::<StoredCookieExclusionReason>::new()
    );
    assert_eq!(
        secret.exclusion_reasons,
        vec![StoredCookieExclusionReason::HttpOnly]
    );
}

#[test]
fn document_cookie_access_report_projects_schemeful_downgrade_like_chromium_script_set() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("https://example.com/app/index.html");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("http://example.com/frame.html"))
        .with_top_frame_origin_url(&parse("http://example.com/frame.html"));

    store.store_response_headers(
        &document_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn document_cookie_access_report_treats_wss_and_https_as_schemefully_same_site_like_chromium_script_set()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("wss://api.example.com/socket");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("https://app.example.com/index.html"))
        .with_top_frame_origin_url(&parse("https://app.example.com/index.html"));

    store.store_response_headers(
        &parse("https://api.example.com/socket"),
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
}

#[test]
fn document_cookie_access_report_projects_cross_site_when_site_for_cookies_is_cross_site_like_chromium_script_set()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("https://example.com/app/index.html");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("https://other.test/frame.html"))
        .with_top_frame_origin_url(&parse("https://other.test/frame.html"));

    store.store_response_headers(
        &document_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/app; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be represented");

    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn document_cookie_access_report_treats_ws_and_http_as_schemefully_same_site_like_chromium_script_set()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("ws://api.example.com/socket");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("http://app.example.com/index.html"))
        .with_top_frame_origin_url(&parse("http://app.example.com/index.html"));

    store.store_response_headers(
        &parse("http://api.example.com/socket"),
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
}

#[test]
fn document_cookie_access_report_treats_local_file_urls_as_same_site_like_chromium_site_for_cookies()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("file:///C:/app/index.html");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("file:///etc/shadow"))
        .with_top_frame_origin_url(&parse("file:///etc/shadow"));

    store.store_response_headers(
        &document_url,
        &[("set-cookie".to_owned(), "lax=1; Path=/".to_owned())],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
}

#[test]
fn document_cookie_access_report_treats_nonlocal_file_urls_as_cross_site_like_chromium_site_for_cookies()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("file:///C:/app/index.html");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("file://nonlocal/file.txt"))
        .with_top_frame_origin_url(&parse("file://nonlocal/file.txt"));

    store.store_response_headers(
        &document_url,
        &[("set-cookie".to_owned(), "lax=1; Path=/".to_owned())],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be represented");

    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn document_cookie_access_report_treats_secure_blob_urls_as_same_site_like_chromium_site_for_cookies()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:https://example.org/9115d58c-bcda-ff47-86e5-083e9a2153041");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("https://sub.example.org/resource"))
        .with_top_frame_origin_url(&parse("https://sub.example.org/resource"));

    store.store_response_headers(
        &parse("https://example.org/resource"),
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
}

#[test]
fn document_cookie_access_report_treats_insecure_blob_urls_as_schemeful_cross_site_like_chromium_site_for_cookies()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:http://example.org/9115d58c-bcda-ff47-86e5-083e9a2153041");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("https://sub.example.org/resource"))
        .with_top_frame_origin_url(&parse("https://sub.example.org/resource"));

    store.store_response_headers(
        &parse("http://example.org/resource"),
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn document_cookie_access_report_treats_secure_blob_urls_with_insecure_site_for_cookies_as_schemeful_cross_site_like_chromium_site_for_cookies()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:https://example.org/9115d58c-bcda-ff47-86e5-083e9a2153041");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("http://sub.example.org/resource"))
        .with_top_frame_origin_url(&parse("http://sub.example.org/resource"));

    store.store_response_headers(
        &parse("https://example.org/resource"),
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn document_cookie_reads_for_blob_file_urls_like_chromium_site_for_cookies() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:file:///C:/app/index.html");

    store.store_response_headers(
        &parse("file:///C:/app/index.html"),
        &[("set-cookie".to_owned(), "lax=1; Path=/".to_owned())],
    );

    assert_eq!(store.document_cookie(&document_url), "lax=1");
}

#[test]
fn document_cookie_reads_for_secure_blob_urls_like_underlying_origin() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:https://example.org/9115d58c-bcda-ff47-86e5-083e9a2153041");

    store.store_response_headers(
        &parse("https://example.org/resource"),
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
        )],
    );

    assert_eq!(store.document_cookie(&document_url), "lax=1");
}

#[test]
fn document_cookie_reads_for_insecure_blob_urls_like_underlying_origin() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:http://example.org/9115d58c-bcda-ff47-86e5-083e9a2153041");

    store.store_response_headers(
        &parse("http://example.org/resource"),
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; SameSite=Lax".to_owned(),
        )],
    );

    assert_eq!(store.document_cookie(&document_url), "lax=1");
}

#[test]
fn document_cookie_access_report_treats_blob_file_urls_as_same_site_like_chromium_site_for_cookies()
{
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:file:///C:/app/index.html");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("file:///etc/shadow"))
        .with_top_frame_origin_url(&parse("file:///etc/shadow"));

    store.store_response_headers(
        &parse("file:///C:/app/index.html"),
        &[("set-cookie".to_owned(), "lax=1; Path=/".to_owned())],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
}

#[test]
fn document_cookie_access_report_treats_blob_file_urls_with_nonlocal_site_for_cookies_as_cross_site_like_chromium_site_for_cookies()
 {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("blob:file:///C:/app/index.html");
    let browser_context = super::BrowserCookieFacadeContext::default()
        .with_site_for_cookies_url(&parse("file://nonlocal/file.txt"))
        .with_top_frame_origin_url(&parse("file://nonlocal/file.txt"));

    store.store_response_headers(
        &parse("file:///C:/app/index.html"),
        &[("set-cookie".to_owned(), "lax=1; Path=/".to_owned())],
    );

    let report = store.document_cookie_access_report_with_context(&document_url, &browser_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be represented");

    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn observation_request_access_report_does_not_touch_access_time() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "theme=dark; Path=/app".to_owned())],
    );

    let before = store
        .full_core
        .get("example.com", "/app", "theme")
        .expect("cookie should exist")
        .last_access_index();

    let report = store.observe_cookie_access_report_for_request(
        &url,
        NetworkCookieRequestContext::subresource("GET"),
    );
    assert_eq!(report.included_cookies.len(), 1);

    let after = store
        .full_core
        .get("example.com", "/app", "theme")
        .expect("cookie should still exist")
        .last_access_index();
    assert_eq!(after, before);
}

#[test]
fn network_cookie_header_reflects_http_writes() {
    let mut store = BrowserCookieStore::default();
    let response_url = parse("https://example.com/app/index.html");
    let request_url = parse("https://example.com/app/panel");

    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "theme=dark; Path=/app; Secure".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&request_url),
        Some("theme=dark".to_owned())
    );
}

#[test]
fn rejected_http_cookie_is_not_stored() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/");

    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "__Secure-sid=1; Path=/".to_owned())],
    );

    assert_eq!(store.cookie_header(&url), None);
}

#[test]
fn store_cookie_maps_structured_core_result_back_to_wrapper_bool() {
    let mut store = BrowserCookieStore::default();
    let secure_url = parse("https://example.com/");
    let insecure_url = parse("http://example.com/");

    let accepted_cookie = StoreCookie::parse("sid=1; Path=/; Secure", &secure_url)
        .unwrap()
        .into_owned();
    assert!(store.store_cookie(
        &secure_url,
        accepted_cookie,
        CookieSource::Http,
        CookiePriority::Medium,
    ));

    let rejected_cookie = StoreCookie::parse("__Secure-bad=1; Path=/", &secure_url)
        .unwrap()
        .into_owned();
    assert!(!store.store_cookie(
        &secure_url,
        rejected_cookie,
        CookieSource::Http,
        CookiePriority::Medium,
    ));

    let deleting_cookie = StoreCookie::parse("sid=gone; Path=/; Max-Age=0; Secure", &secure_url)
        .unwrap()
        .into_owned();
    assert!(store.store_cookie(
        &secure_url,
        deleting_cookie,
        CookieSource::Http,
        CookiePriority::Medium,
    ));

    let secure_from_insecure = StoreCookie::parse("other=1; Path=/; Secure", &insecure_url)
        .unwrap()
        .into_owned();
    assert!(!store.store_cookie(
        &insecure_url,
        secure_from_insecure,
        CookieSource::Http,
        CookiePriority::Medium,
    ));
}

#[test]
fn network_cookie_header_reflects_document_writes() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");
    let request_url = parse("https://example.com/app/panel");

    store.set_document_cookie(&url, "sid=client; Path=/app");

    assert_eq!(store.document_cookie(&url), "sid=client");
    assert_eq!(
        store.cookie_header(&request_url),
        Some("sid=client".to_owned())
    );
}

#[test]
fn network_cookie_header_reflects_http_removals() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");
    let request_url = parse("https://example.com/app/panel");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=server; Path=/app; Secure".to_owned(),
        )],
    );
    assert_eq!(
        store.cookie_header(&request_url),
        Some("sid=server".to_owned())
    );

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=gone; Path=/app; Secure; Max-Age=0".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&request_url), None);
}

#[test]
fn document_cookie_utc_expires_does_not_leave_empty_host_only_shadow() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("https://zhuanlan.zhihu.com/p/23888614724");

    store.set_document_cookie(
        &document_url,
        "__zse_ck=real; Domain=.zhihu.com; Path=/; SameSite=None; Secure",
    );
    assert_eq!(
        store.cookie_header(&document_url),
        Some("__zse_ck=real".to_owned())
    );

    store.set_document_cookie(
        &document_url,
        "__zse_ck=; Expires=Mon, 20 Sep 1970 00:00:00 UTC; Path=/",
    );

    assert_eq!(
        store.cookie_header(&document_url),
        Some("__zse_ck=real".to_owned())
    );
    assert_eq!(store.document_cookie(&document_url), "__zse_ck=real");
    let cookies = store.cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "__zse_ck");
    assert_eq!(cookies[0].value, "real");
    assert_eq!(cookies[0].domain, "zhihu.com");
    assert!(!cookies[0].host_only);
}

#[test]
fn document_cookie_expires_without_timezone_is_treated_as_gmt() {
    let mut store = BrowserCookieStore::default();
    let document_url = parse("https://example.com/cookies");

    store.set_document_cookie(
        &document_url,
        "foo=bar; Expires=Tue, 09 Jun 2037 19:21:05; Path=/",
    );

    assert_eq!(store.document_cookie(&document_url), "foo=bar");
    let cookies = store.cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "foo");
    assert_eq!(cookies[0].value, "bar");
    assert!(
        cookies[0].expires.is_some(),
        "WPT document.cookie expires dates without an explicit timezone should remain persistent"
    );
}

#[test]
fn clear_removes_all_network_visible_cookies() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=server; Path=/app; Secure".to_owned(),
        )],
    );
    store.set_document_cookie(&url, "theme=dark; Path=/app");

    store.clear();

    assert_eq!(store.cookie_header(&url), None);
    assert_eq!(store.document_cookie(&url), "");
}

#[test]
fn sites_with_cookies_uses_registrable_site_keys() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "a=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://cdn.example.com/assets/index.html"),
        &[("set-cookie".to_owned(), "b=1; Path=/assets".to_owned())],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[("set-cookie".to_owned(), "c=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://127.0.0.1/app/index.html"),
        &[("set-cookie".to_owned(), "d=1; Path=/app".to_owned())],
    );

    assert_eq!(
        store.sites_with_cookies(),
        vec![
            "127.0.0.1".to_owned(),
            "example.com".to_owned(),
            "foo.co.uk".to_owned(),
        ]
    );
}

#[test]
fn cookie_site_data_counts_cookies_per_registrable_site() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "a=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://cdn.example.com/assets/index.html"),
        &[("set-cookie".to_owned(), "b=1; Path=/assets".to_owned())],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[
            ("set-cookie".to_owned(), "c=1; Path=/app".to_owned()),
            ("set-cookie".to_owned(), "d=1; Path=/app".to_owned()),
        ],
    );
    store.store_response_headers(
        &parse("https://127.0.0.1/app/index.html"),
        &[("set-cookie".to_owned(), "e=1; Path=/app".to_owned())],
    );

    let site_data = store.cookie_site_data();

    assert_eq!(
        site_data,
        vec![
            site_summary("127.0.0.1", 1, 0),
            site_summary("example.com", 2, 0),
            site_summary("foo.co.uk", 2, 0),
        ]
    );
}

#[test]
fn cookie_site_data_persistent_scope_excludes_session_cookies() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persistent=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_site_data_with_scope(super::CookieSiteDataScope::Live),
        vec![site_summary("example.com", 2, 1)]
    );
    assert_eq!(
        store.cookie_site_data_with_scope(super::CookieSiteDataScope::Persistent),
        vec![site_summary("example.com", 1, 1)]
    );
}

#[test]
fn clear_cookies_for_sites_removes_all_matching_registrable_sites() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "a=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://cdn.example.com/assets/index.html"),
        &[("set-cookie".to_owned(), "b=1; Path=/assets".to_owned())],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[("set-cookie".to_owned(), "c=1; Path=/app".to_owned())],
    );

    let removed = store.clear_cookies_for_sites(&["sub.example.com"]);

    assert_eq!(removed, 2);
    assert_eq!(store.sites_with_cookies(), vec!["foo.co.uk".to_owned()]);
    assert_eq!(
        store.cookie_header(&parse("https://app.example.com/app/panel")),
        None
    );
    assert_eq!(
        store.cookie_header(&parse("https://foo.co.uk/app/panel")),
        Some("c=1".to_owned())
    );
}

#[test]
fn clear_cookies_for_sites_report_projects_replaced_and_remaining_state() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://cdn.example.com/assets/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persistent=1; Path=/assets; Max-Age=3600".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "other=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let report = store.clear_cookies_for_sites_with_report(&["deep.example.com"]);

    assert_eq!(report.requested_sites, vec!["example.com".to_owned()]);
    assert_eq!(report.removed_cookie_count, 2);
    assert_eq!(
        report.replaced_state.live_site_data,
        vec![site_summary("example.com", 2, 1)]
    );
    assert_eq!(
        report.state_diff.live_site_changes,
        vec![site_change("example.com", Some((2, 1)), None)]
    );
    assert_eq!(
        report.state_diff.persistent_site_changes,
        vec![site_change("example.com", Some((1, 1)), None)]
    );
    assert!(report.resulting_state.live_site_data.is_empty());
    assert_eq!(
        store.cookie_site_data(),
        vec![site_summary("foo.co.uk", 1, 1)]
    );
}

#[test]
fn preview_clear_cookies_for_sites_reports_targeted_removal_without_mutation() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://cdn.example.com/assets/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persistent=1; Path=/assets; Max-Age=3600".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "other=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let before_generation = store.document_cookie_generation();
    let before_cookie = store.document_cookie(&parse("https://app.example.com/app/index.html"));

    let preview = store.preview_clear_cookies_for_sites(&["deep.example.com"]);

    assert_eq!(preview.requested_sites, vec!["example.com".to_owned()]);
    assert_eq!(preview.would_remove_cookie_count, 2);
    assert_eq!(
        preview.replaced_state.store_generation,
        Some(before_generation)
    );
    assert_eq!(preview.resulting_state.store_generation, None);
    assert_eq!(
        preview.state_diff.live_site_changes,
        vec![site_change("example.com", Some((2, 1)), None)]
    );
    assert_eq!(
        preview.state_diff.persistent_site_changes,
        vec![site_change("example.com", Some((1, 1)), None)]
    );
    assert_eq!(
        preview.replaced_state.live_site_data,
        vec![site_summary("example.com", 2, 1)]
    );
    assert_eq!(
        preview.resulting_state.live_site_data,
        vec![site_summary("foo.co.uk", 1, 1)]
    );
    assert_eq!(store.document_cookie_generation(), before_generation);
    assert_eq!(
        store.document_cookie(&parse("https://app.example.com/app/index.html")),
        before_cookie
    );
}

#[test]
fn preview_clear_cookies_for_sites_with_persistent_scope_keeps_session_slice() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persist=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "other=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let preview = store.preview_clear_cookies_for_sites_with_scope(
        &["deep.example.com"],
        super::CookieSiteDataClearScope::Persistent,
    );

    assert_eq!(preview.scope, super::CookieSiteDataClearScope::Persistent);
    assert_eq!(preview.would_remove_cookie_count, 1);
    assert_eq!(
        preview.state_diff.live_site_changes,
        vec![site_change("example.com", Some((2, 1)), Some((1, 0)))]
    );
    assert_eq!(
        preview.state_diff.persistent_site_changes,
        vec![site_change("example.com", Some((1, 1)), None)]
    );
    assert_eq!(
        preview.resulting_state.live_site_data,
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 1),
        ]
    );
}

#[test]
fn clear_cookies_for_sites_with_session_scope_preserves_persistent_cookie() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persist=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let report = store.clear_cookies_for_sites_with_scope_and_report(
        &["deep.example.com"],
        super::CookieSiteDataClearScope::Session,
    );

    assert_eq!(report.scope, super::CookieSiteDataClearScope::Session);
    assert_eq!(report.removed_cookie_count, 1);
    assert_eq!(
        report.state_diff.live_site_changes,
        vec![site_change("example.com", Some((2, 1)), Some((1, 1)))]
    );
    assert!(report.state_diff.persistent_site_changes.is_empty());
    assert_eq!(
        store.document_cookie(&parse("https://app.example.com/app/index.html")),
        "persist=1"
    );
    assert_eq!(
        store.cookie_site_data(),
        vec![site_summary("example.com", 1, 1)]
    );
}

#[test]
fn preview_clear_cookie_store_with_persistent_scope_keeps_session_sites() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persist=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "other=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let preview = store.preview_clear_with_scope(super::CookieSiteDataClearScope::Persistent);

    assert_eq!(preview.scope, super::CookieSiteDataClearScope::Persistent);
    assert_eq!(preview.would_remove_cookie_count, 2);
    assert_eq!(
        preview.resulting_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
    assert!(
        preview
            .state_diff
            .persistent_site_changes
            .iter()
            .any(|change| change.name == "foo.co.uk")
    );
    assert_eq!(preview.target, super::CookieStorageClearTarget::WholeStore);
}

#[test]
fn clear_cookie_store_with_session_scope_preserves_persistent_store_state() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persist=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[("set-cookie".to_owned(), "other=1; Path=/app".to_owned())],
    );

    let report = store.clear_with_scope_and_report(super::CookieSiteDataClearScope::Session);

    assert_eq!(report.scope, super::CookieSiteDataClearScope::Session);
    assert_eq!(report.removed_cookie_count, 2);
    assert_eq!(report.target, super::CookieStorageClearTarget::WholeStore);
    assert_eq!(
        report.resulting_state.live_site_data,
        vec![site_summary("example.com", 1, 1)]
    );
    assert_eq!(
        report.resulting_state.persistent_site_data,
        vec![site_summary("example.com", 1, 1)]
    );
}

#[test]
fn preview_clear_with_registrable_site_target_projects_targeted_clear_shape() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "other=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let preview = store.preview_clear_with_scope_and_target(
        super::CookieStorageClearTarget::RegistrableSites(vec!["deep.example.com".to_owned()]),
        super::CookieSiteDataClearScope::All,
    );

    assert_eq!(
        preview.target,
        super::CookieStorageClearTarget::RegistrableSites(vec!["example.com".to_owned()])
    );
    assert_eq!(preview.scope, super::CookieSiteDataClearScope::All);
    assert_eq!(preview.would_remove_cookie_count, 1);
    assert_eq!(
        preview.replaced_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
    assert_eq!(
        preview.resulting_state.live_site_data,
        vec![site_summary("foo.co.uk", 1, 1)]
    );
}

#[test]
fn clear_with_registrable_site_target_projects_targeted_clear_report() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://cdn.example.com/assets/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persist=1; Path=/assets; Max-Age=3600".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[("set-cookie".to_owned(), "other=1; Path=/app".to_owned())],
    );

    let report = store.clear_with_scope_and_target_report(
        super::CookieStorageClearTarget::RegistrableSites(vec!["deep.example.com".to_owned()]),
        super::CookieSiteDataClearScope::Persistent,
    );

    assert_eq!(
        report.target,
        super::CookieStorageClearTarget::RegistrableSites(vec!["example.com".to_owned()])
    );
    assert_eq!(report.scope, super::CookieSiteDataClearScope::Persistent);
    assert_eq!(report.removed_cookie_count, 1);
    assert_eq!(
        report.resulting_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
    assert_eq!(
        store.cookie_site_data(),
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 0),
        ]
    );
}

#[test]
fn preview_site_data_operation_clear_uses_generic_targeted_clear_seam() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "other=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let preview = store
        .preview_site_data_operation(&super::CookieSiteDataOperation::Clear {
            target: super::CookieStorageClearTarget::RegistrableSites(vec![
                "deep.example.com".to_owned(),
            ]),
            scope: super::CookieSiteDataClearScope::All,
        })
        .expect("clear preview should succeed");

    let super::CookieSiteDataOperationPreviewReport::Clear(report) = preview;
    assert_eq!(
        report.target,
        super::CookieStorageClearTarget::RegistrableSites(vec!["example.com".to_owned()])
    );
    assert_eq!(report.would_remove_cookie_count, 1);
    assert_eq!(
        report.replaced_state.live_site_data,
        vec![site_summary("example.com", 1, 0)]
    );
}

#[test]
fn cookie_storage_state_snapshot_distinguishes_live_and_persistent_views() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persistent=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let snapshot = store.cookie_storage_state_snapshot();

    assert_eq!(
        snapshot.store_generation,
        Some(store.document_cookie_generation())
    );
    assert_eq!(snapshot.live_cookie_count, 2);
    assert_eq!(snapshot.persistent_cookie_count, 1);
    assert_eq!(
        snapshot.live_site_data,
        vec![
            site_summary("example.com", 1, 0),
            site_summary("foo.co.uk", 1, 1),
        ]
    );
    assert_eq!(
        snapshot.persistent_site_data,
        vec![site_summary("foo.co.uk", 1, 1)]
    );
}

#[test]
fn cookie_storage_state_snapshot_for_sites_filters_live_and_persistent_views() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://app.example.com/app/index.html"),
        &[("set-cookie".to_owned(), "session=1; Path=/app".to_owned())],
    );
    store.store_response_headers(
        &parse("https://cdn.example.com/assets/index.html"),
        &[(
            "set-cookie".to_owned(),
            "persistent=1; Path=/assets; Max-Age=3600".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://foo.co.uk/app/index.html"),
        &[(
            "set-cookie".to_owned(),
            "other=1; Path=/app; Max-Age=3600".to_owned(),
        )],
    );

    let snapshot = store.cookie_storage_state_snapshot_for_sites(&["deep.example.com"]);

    assert_eq!(snapshot.live_cookie_count, 2);
    assert_eq!(snapshot.persistent_cookie_count, 1);
    assert_eq!(
        snapshot.live_site_data,
        vec![site_summary("example.com", 2, 1)]
    );
    assert_eq!(
        snapshot.persistent_site_data,
        vec![site_summary("example.com", 1, 1)]
    );
}

#[test]
fn cookies_enumeration_reads_priority_and_source_metadata_from_full_core() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com:8443/app/index.html");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=server; Path=/app; Secure; HttpOnly; Priority=High".to_owned(),
        )],
    );

    // Priority and source metadata used to live in wrapper-local metadata. Now they are derived
    // from the canonical forked core, so enumeration must keep reporting them without any local
    // mutation path.
    let enumerated = store.cookies();
    assert_eq!(enumerated.len(), 1);
    assert_eq!(enumerated[0].value, "server");
    assert!(enumerated[0].http_only);
    assert!(enumerated[0].secure);
    assert_eq!(enumerated[0].priority, Some(CookiePriority::High));
    assert_eq!(enumerated[0].source_port, 8443);
}

#[test]
fn cookies_enumeration_drops_metadata_entries_missing_from_full_core() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=server; Path=/app; Secure".to_owned(),
        )],
    );
    assert_eq!(store.cookies().len(), 1);

    // Externally visible cookie enumeration now comes entirely from the canonical core.
    // Clearing it must leave nothing behind in the browser-facing projection.
    store.full_core.clear();

    assert!(store.cookies().is_empty());
}

#[test]
fn cdp_upsert_updates_network_visible_cookie_header() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/");

    store.upsert(
        StoredCookie {
            name: "sid".to_owned(),
            value: "cdp".to_owned(),
            domain: "example.com".to_owned(),
            host_only: true,
            path: "/".to_owned(),
            secure: false,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::Unspecified,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::Unset,
            source_port: -1,
            creation_index: 999,
            last_access_index: 999,
        },
        CookieSource::Cdp,
    );

    assert_eq!(store.cookie_header(&url), Some("sid=cdp".to_owned()));
}

#[test]
fn cdp_upsert_host_only_cookie_preserves_explicit_domain_without_request_hint() {
    let mut store = BrowserCookieStore::default();
    let restored_url = parse("https://restored.example/");
    let other_url = parse("https://example.com/");

    store.upsert(
        StoredCookie {
            name: "sid".to_owned(),
            value: "restored".to_owned(),
            domain: "restored.example".to_owned(),
            host_only: true,
            path: "/".to_owned(),
            secure: false,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::Unspecified,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::Unset,
            source_port: -1,
            creation_index: 999,
            last_access_index: 999,
        },
        CookieSource::Cdp,
    );

    assert_eq!(
        store.cookie_header(&restored_url),
        Some("sid=restored".to_owned())
    );
    assert_eq!(store.cookie_header(&other_url), None);
}

#[test]
fn delete_cookies_updates_network_visible_cookie_header() {
    let mut store = BrowserCookieStore::default();
    let response_url = parse("https://sub.example.com/app/index.html");
    let request_url = parse("https://sub.example.com/app/panel");

    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "shared=1; Domain=example.com; Path=/app; Secure".to_owned(),
        )],
    );
    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "host=1; Path=/app; Secure".to_owned(),
        )],
    );
    assert_eq!(
        store.cookie_header(&request_url),
        Some("shared=1; host=1".to_owned())
    );

    let removed = store.delete_cookies(Some("shared"), None, None, Some("deep.sub.example.com"));

    assert_eq!(removed, 1);
    assert_eq!(store.cookie_header(&request_url), Some("host=1".to_owned()));
}

#[test]
fn delete_cookies_url_host_filter_uses_full_core_host_only_state() {
    let mut store = BrowserCookieStore::default();
    let response_url = parse("https://example.com/app/index.html");

    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "host=1; Path=/app; Secure".to_owned(),
        )],
    );
    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "shared=1; Domain=example.com; Path=/app; Secure".to_owned(),
        )],
    );

    let removed = store.delete_cookies(None, None, None, Some("deep.sub.example.com"));

    assert_eq!(removed, 1);
    let cookies = store.cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "host");
    assert!(cookies[0].host_only);
}

#[test]
fn dot_prefixed_ip_domain_cookie_is_accepted_as_host_cookie_like_chromium() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://192.0.2.3/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=.192.0.2.3; Path=/".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), Some("sid=1".to_owned()));
    let cookies = store.cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].domain, "192.0.2.3");
    assert!(cookies[0].host_only);
}

#[test]
fn trailing_dot_cookie_domain_is_rejected_like_chromium_cookie_util() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://foo.com/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=.foo.com..; Path=/".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), None);
    assert!(store.cookies().is_empty());
}

#[test]
fn percent_encoded_cookie_domain_is_rejected_like_chromium_cookie_util() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://a.test/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=a%2Etest; Path=/".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), None);
    assert!(store.cookies().is_empty());
}

#[test]
fn uncanonicalizable_cookie_domain_is_rejected_like_chromium_cookie_util() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://a.test/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=a^test; Path=/".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), None);
    assert!(store.cookies().is_empty());
}

#[test]
fn dot_prefixed_public_suffix_identical_host_becomes_host_only_like_chromium() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://github.io/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "hostonly=1; Domain=.github.io; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), Some("hostonly=1".to_owned()));
    assert_eq!(store.cookie_header(&parse("https://foo.github.io/")), None);
    let cookies = store.cookies();
    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].host_only);
    assert_eq!(cookies[0].domain, "github.io");
}

#[test]
fn dot_prefixed_public_suffix_identical_host_becomes_host_only_for_gov_uk_like_chromium() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://gov.uk/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "hostonly=1; Domain=.gov.uk; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), Some("hostonly=1".to_owned()));
    assert_eq!(store.cookie_header(&parse("https://nhs.gov.uk/")), None);
    let cookies = store.cookies();
    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].host_only);
    assert_eq!(cookies[0].domain, "gov.uk");
}

#[test]
fn noncanonical_public_suffix_identical_host_becomes_host_only_for_gov_uk_like_chromium() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://gov.uk/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "hostonly=1; Domain=GoV.Uk; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), Some("hostonly=1".to_owned()));
    assert_eq!(store.cookie_header(&parse("https://nhs.gov.uk/")), None);
    let cookies = store.cookies();
    assert_eq!(cookies.len(), 1);
    assert!(cookies[0].host_only);
    assert_eq!(cookies[0].domain, "gov.uk");
}

#[test]
fn parent_domain_attribute_is_accepted_for_subdomain_request_host_like_chromium() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://mail.globex.com/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=globex.com; Path=/".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("http://portal.globex.com/")),
        Some("sid=1".to_owned())
    );
}

#[test]
fn subdomain_attribute_is_rejected_for_parent_request_host_like_chromium() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("http://globex.com/"),
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=mail.globex.com; Path=/".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&parse("http://globex.com/")), None);
    assert!(store.cookies().is_empty());
}

#[test]
fn substring_but_not_subdomain_domain_attribute_is_rejected_like_chromium() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("http://myglobex.com/"),
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=globex.com; Path=/".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&parse("http://myglobex.com/")), None);
    assert!(store.cookies().is_empty());
}

#[test]
fn trailing_dot_domain_attribute_mismatch_is_rejected_like_chromium_cookie_util() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://foo.com/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=.foo.com..; Path=/".to_owned(),
        )],
    );
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=.foo.com.; Path=/".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), None);
    assert!(store.cookies().is_empty());
}

#[test]
fn invalid_ip_subdomain_domain_attribute_is_rejected_like_chromium_cookie_util() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("http://192.0.2.3/"),
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=192; Path=/".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("http://0.0.16.0/0000000"),
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=00000000; Path=/".to_owned(),
        )],
    );

    assert!(store.cookies().is_empty());
}

#[test]
fn unknown_registry_identical_domain_attribute_is_accepted_like_chromium_cookie_util() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://qjz9/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Domain=qjz9; Path=/".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), Some("sid=1".to_owned()));
    assert_eq!(
        store.cookie_header(&parse("http://foo.qjz9/")),
        Some("sid=1".to_owned())
    );
}

#[test]
fn secure_cookie_from_insecure_origin_is_ignored() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("http://example.com/login"),
        &[(
            "set-cookie".to_owned(),
            "sid=secure; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&parse("https://example.com/")), None);
}

#[test]
fn secure_prefix_requires_secure_attribute() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com/"),
        &[("set-cookie".to_owned(), "__Secure-sid=1; Path=/".to_owned())],
    );

    assert_eq!(store.cookie_header(&parse("https://example.com/")), None);
}

#[test]
fn host_prefix_requires_host_only_secure_and_explicit_root_path() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/account/index.html");

    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "__Host-a=1; Secure".to_owned())],
    );
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "__Host-b=1; Secure; Path=/".to_owned(),
        )],
    );
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "__Host-c=1; Secure; Path=/; Domain=example.com".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("__Host-b=1".to_owned())
    );
}

#[test]
fn http_prefix_requires_secure_and_http_only() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "__Http-token=1; Path=/; Secure".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "__Http-token=2; Path=/; Secure; HttpOnly".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("__Http-token=2".to_owned())
    );
    assert_eq!(store.document_cookie(&parse("https://example.com/")), "");
}

#[test]
fn host_http_prefix_requires_host_only_secure_http_only_and_explicit_root_path() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "__Host-Http-token=1; Path=/; Secure".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "__Host-Http-token=2; Path=/; Secure; HttpOnly; Domain=example.com".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "__Host-Http-token=3; Path=/; Secure; HttpOnly".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("__Host-Http-token=3".to_owned())
    );
}

#[test]
fn empty_name_cookie_cannot_smuggle_protected_prefixes_in_value() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "=__Secure-token; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&parse("https://example.com/")), None);
}

#[test]
fn same_site_none_requires_secure() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "cross=1; Path=/; SameSite=None".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "cross=1; Path=/; SameSite=None; Secure".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("cross=1".to_owned())
    );
}

#[test]
fn insecure_cookie_cannot_overlay_existing_secure_cookie() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com/login"),
        &[(
            "set-cookie".to_owned(),
            "sid=secure; Domain=example.com; Path=/login; Secure".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("http://sub.example.com/login"),
        &[(
            "set-cookie".to_owned(),
            "sid=insecure; Domain=example.com; Path=/login".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/login")),
        Some("sid=secure".to_owned())
    );
    assert_eq!(
        store.cookie_header(&parse("https://sub.example.com/login")),
        Some("sid=secure".to_owned())
    );
}

#[test]
fn secure_overlay_guard_reads_canonical_core() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com/login"),
        &[(
            "set-cookie".to_owned(),
            "sid=secure; Domain=example.com; Path=/login; Secure".to_owned(),
        )],
    );

    // Overlay protection is sourced from the canonical core entry even though the wrapper no
    // longer keeps any mirrored cookie payload state.
    store.store_response_headers(
        &parse("http://sub.example.com/login"),
        &[(
            "set-cookie".to_owned(),
            "sid=insecure; Domain=example.com; Path=/login".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://sub.example.com/login")),
        Some("sid=secure".to_owned())
    );
}

#[test]
fn cookie_header_keeps_original_creation_order_for_same_path_length() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/");

    store.store_response_headers(&url, &[("set-cookie".to_owned(), "b=2; Path=/".to_owned())]);
    store.store_response_headers(&url, &[("set-cookie".to_owned(), "a=1; Path=/".to_owned())]);

    assert_eq!(store.cookie_header(&url), Some("b=2; a=1".to_owned()));
}

#[test]
fn response_header_reports_project_sanitized_attribute_warnings() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/app/index.html");
    let oversized_domain = format!("{}.com", "a".repeat(1021));
    let invalid_path = "/\u{7f}invalid";

    let reports = store.store_response_headers_with_reports(
        &url,
        &[(
            "set-cookie".to_owned(),
            format!("sid=1; Domain={oversized_domain}; Path={invalid_path}; Secure"),
        )],
    );

    assert_eq!(reports.len(), 1);
    assert!(reports[0].is_accepted());
    assert_eq!(
        reports[0].warning_reasons,
        vec![
            StoredCookieSetWarningReason::DomainAttributeIgnored,
            StoredCookieSetWarningReason::PathAttributeIgnored,
        ]
    );
    assert_eq!(
        reports[0].effective_same_site,
        Some(StoredCookieEffectiveSameSite::NoRestriction)
    );
}

#[test]
fn response_header_reports_project_secure_access_warning_for_localhost_http() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://localhost/app/index.html");

    let reports = store.store_response_headers_with_reports(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Path=/app; Secure".to_owned(),
        )],
    );

    assert_eq!(reports.len(), 1);
    assert!(reports[0].is_accepted());
    assert_eq!(
        reports[0].warning_reasons,
        vec![StoredCookieSetWarningReason::SecureAccessGrantedNonCryptographic]
    );
}

#[test]
fn request_access_report_projects_schemeful_same_site_warning() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/foo/bar");

    store.store_response_headers(
        &url,
        &[
            (
                "set-cookie".to_owned(),
                "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
            ),
            (
                "set-cookie".to_owned(),
                "none=1; Path=/foo; Secure; SameSite=None".to_owned(),
            ),
        ],
    );

    let report = store.cookie_access_report_for_request(
        &url,
        NetworkCookieRequestContext::subresource("GET").with_site_context(NetworkSiteContext::new(
            NetworkSameSiteContext::SameSiteStrict,
            NetworkSameSiteContext::CrossSite,
        )),
    );

    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");
    let none = find_cookie(&report, "none").expect("none cookie should be present");

    assert!(
        report
            .excluded_cookies
            .iter()
            .any(|entry| entry.cookie.name == "strict")
    );
    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.warning_reasons,
        vec![StoredCookieWarningReason::StrictCrossDowngradeStrictSameSite]
    );
    assert_eq!(
        strict.effective_same_site,
        StoredCookieEffectiveSameSite::Strict
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Get
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::NoRedirect
    );

    assert!(
        report
            .included_cookies
            .iter()
            .any(|entry| entry.cookie.name == "none")
    );
    assert!(none.warning_reasons.is_empty());
}

#[test]
fn request_access_report_projects_lax_schemeful_same_site_warning() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/foo/bar");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/foo; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &url,
        NetworkCookieRequestContext::subresource("GET")
            .with_site_context(NetworkSiteContext::new(
                NetworkSameSiteContext::SameSiteStrict,
                NetworkSameSiteContext::CrossSite,
            ))
            .with_site_context_metadata(NetworkSiteContextMetadata::schemeful_only(
                false,
                Some(NetworkSameSiteContextDowngradeType::StrictToCross),
            )),
    );

    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");
    assert_eq!(
        lax.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.warning_reasons,
        vec![StoredCookieWarningReason::StrictCrossDowngradeLaxSameSite]
    );
    assert_eq!(lax.effective_same_site, StoredCookieEffectiveSameSite::Lax);
}

#[test]
fn request_access_report_projects_secure_access_warning_for_localhost_http() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://localhost/app/index.html");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Path=/app; Secure".to_owned(),
        )],
    );

    let report = store
        .cookie_access_report_for_request(&url, NetworkCookieRequestContext::subresource("GET"));
    let sid = find_cookie(&report, "sid").expect("sid cookie should be present");

    assert!(sid.exclusion_reasons.is_empty());
    assert_eq!(
        sid.warning_reasons,
        vec![StoredCookieWarningReason::SecureAccessGrantedNonCryptographic]
    );
}

#[test]
fn request_access_report_projects_schemeful_only_redirect_metadata() {
    let mut store = BrowserCookieStore::default();
    let url = parse("http://example.com/foo/bar");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &url,
        NetworkCookieRequestContext::top_level_navigation("GET")
            .with_site_context(NetworkSiteContext::new(
                NetworkSameSiteContext::SameSiteStrict,
                NetworkSameSiteContext::SameSiteLax,
            ))
            .with_site_context_metadata(NetworkSiteContextMetadata::schemeful_only(
                true,
                Some(NetworkSameSiteContextDowngradeType::StrictToLax),
            )),
    );

    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(strict.same_site_context_downgrade_type, None);
    assert_eq!(
        strict.schemeful_same_site_context_downgrade_type,
        Some(StoredCookieSameSiteContextDowngradeType::StrictToLax)
    );
    assert_eq!(
        strict.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Get
    );
}

#[test]
fn request_access_report_projects_http_method_and_redirect_type() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://other.test/foo/bar");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &url,
        NetworkCookieRequestContext::top_level_navigation("POST")
            .with_site_context(NetworkSiteContext::cross_site()),
    );

    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");
    assert_eq!(
        strict.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::NoRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::NoRedirect
    );
}

#[test]
fn redirected_top_level_get_with_cross_site_initiator_and_same_site_frame_projects_lax_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let cross_site_initiator_url = parse("https://other.test/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let request_context = NetworkCookieRequestContext::top_level_navigation("GET")
        .with_initiator_url(&original_request_url, &cross_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url)
        .recompute_site_context_for_request(&request_url)
        .with_site_context_metadata_for_redirects(&original_request_url, &request_url);

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn redirected_top_level_post_with_cross_site_initiator_and_same_site_frame_projects_lax_unsafe_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let cross_site_initiator_url = parse("https://other.test/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let request_context = NetworkCookieRequestContext::top_level_navigation("POST")
        .with_initiator_url(&original_request_url, &cross_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url)
        .recompute_site_context_for_request(&request_url)
        .with_site_context_metadata_for_redirects(&original_request_url, &request_url);

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert_eq!(
        lax.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe
    );
    assert_eq!(
        lax.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        lax.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn redirected_top_level_get_with_same_site_initiator_and_frame_projects_partial_same_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://other.test/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::top_level_navigation("GET")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
    );
}

#[test]
fn redirected_top_level_post_with_same_site_initiator_and_frame_projects_partial_same_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://other.test/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::top_level_navigation("POST")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert_eq!(
        lax.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe
    );
    assert_eq!(
        lax.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
    );
    assert_eq!(
        lax.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
    );
}

#[test]
fn redirected_subresource_get_with_same_site_initiator_and_frame_projects_partial_same_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://other.test/start");
    let request_url = parse("https://example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::subresource("GET")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
    );
}

#[test]
fn redirected_subresource_post_with_same_site_initiator_and_frame_projects_partial_same_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://other.test/start");
    let request_url = parse("https://example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::subresource("POST")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
    );
}

#[test]
fn redirected_top_level_get_with_same_site_initiator_and_frame_projects_all_same_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let request_context = NetworkCookieRequestContext::top_level_navigation("GET")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url)
        .recompute_site_context_for_request(&request_url)
        .with_site_context_metadata_for_redirects(&original_request_url, &request_url);

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert!(strict.exclusion_reasons.is_empty());
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect
    );
}

#[test]
fn redirected_top_level_post_with_same_site_initiator_and_frame_projects_all_same_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let request_context = NetworkCookieRequestContext::top_level_navigation("POST")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url)
        .recompute_site_context_for_request(&request_url)
        .with_site_context_metadata_for_redirects(&original_request_url, &request_url);

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert!(strict.exclusion_reasons.is_empty());
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect
    );
}

#[test]
fn redirected_subresource_get_with_same_site_initiator_and_frame_projects_all_same_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::subresource("GET")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert!(strict.exclusion_reasons.is_empty());
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect
    );
}

#[test]
fn redirected_subresource_post_with_same_site_initiator_and_frame_projects_all_same_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::subresource("POST")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert!(strict.exclusion_reasons.is_empty());
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect
    );
}

#[test]
fn redirected_subresource_get_with_cross_site_initiator_and_same_site_frame_projects_cross_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let cross_site_initiator_url = parse("https://other.test/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::subresource("GET")
        .with_initiator_url(&original_request_url, &cross_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn redirected_subresource_post_with_cross_site_initiator_and_same_site_frame_projects_cross_site_redirect_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://api.example.com/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let cross_site_initiator_url = parse("https://other.test/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::subresource("POST")
        .with_initiator_url(&original_request_url, &cross_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn redirected_top_level_get_with_same_site_chain_but_cross_site_final_stays_cross_site_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://other.test/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::top_level_navigation("GET")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn redirected_subresource_get_with_same_site_chain_but_cross_site_final_stays_cross_site_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://other.test/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::subresource("GET")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        strict.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn redirected_top_level_post_with_same_site_chain_but_cross_site_final_stays_cross_site_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://other.test/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::top_level_navigation("POST")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should still be represented");

    assert_eq!(
        lax.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        lax.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn redirected_subresource_post_with_same_site_chain_but_cross_site_final_stays_cross_site_like_chromium()
 {
    let mut store = BrowserCookieStore::default();
    let original_request_url = parse("https://cdn.example.com/start");
    let request_url = parse("https://other.test/final");
    let same_site_frame_url = parse("https://www.example.com/frame.html");
    let same_site_initiator_url = parse("https://app.example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let initial_request_context = NetworkCookieRequestContext::subresource("POST")
        .with_initiator_url(&original_request_url, &same_site_initiator_url)
        .with_site_for_cookies_url(&original_request_url, &same_site_frame_url)
        .with_top_frame_origin_url(&original_request_url, &same_site_frame_url);
    let request_context = advance_cookie_request_context(
        initial_request_context,
        &original_request_url,
        &request_url,
    );

    let report = store.cookie_access_report_for_request(&request_url, request_context);
    let lax = find_cookie(&report, "lax").expect("lax cookie should still be represented");

    assert_eq!(
        lax.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
    assert_eq!(
        lax.schemeful_same_site_context_redirect_type,
        StoredCookieSameSiteRedirectType::CrossSiteRedirect
    );
}

#[test]
fn lowercase_safe_http_method_still_allows_top_level_lax_cookie() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://example.com/foo/bar");
    let initiator_url = parse("https://other.test/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/foo; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::top_level_navigation("get")
            .with_initiator_url(&request_url, &initiator_url),
    );

    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");
    assert!(lax.exclusion_reasons.is_empty());
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
    assert_eq!(
        lax.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Get
    );
    assert_eq!(
        lax.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Get
    );
}

#[test]
fn websocket_secure_scheme_is_schemefully_same_site_with_https_initiator_like_chromium() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("wss://api.example.com/socket");
    let initiator_url = parse("https://app.example.com/index.html");
    let cross_site_initiator = parse("https://other.test/index.html");

    store.store_response_headers(
        &parse("https://api.example.com/socket"),
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let same_site_report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&request_url, &initiator_url),
    );
    let strict = find_cookie(&same_site_report, "strict").expect("strict cookie should be present");
    assert!(strict.exclusion_reasons.is_empty());
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );

    let cross_site_report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&request_url, &cross_site_initiator),
    );
    let strict = find_cookie(&cross_site_report, "strict")
        .expect("strict cookie should still be represented in report");
    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn websocket_insecure_scheme_is_schemefully_same_site_with_http_initiator_like_chromium() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("ws://api.example.com/socket");
    let initiator_url = parse("http://app.example.com/index.html");

    store.store_response_headers(
        &parse("http://api.example.com/socket"),
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&request_url, &initiator_url),
    );
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");
    assert!(strict.exclusion_reasons.is_empty());
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
}

#[test]
fn websocket_insecure_scheme_is_cross_site_with_cross_site_initiator_like_chromium() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("ws://api.example.com/socket");
    let cross_site_initiator = parse("https://other.test/index.html");

    store.store_response_headers(
        &parse("http://api.example.com/socket"),
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&request_url, &cross_site_initiator),
    );
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn top_level_get_request_reports_schemeful_lax_for_cross_scheme_same_site_initiator() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://example.com/foo/bar");
    let initiator_url = parse("http://example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::top_level_navigation("GET")
            .with_initiator_url(&request_url, &initiator_url),
    );
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
}

#[test]
fn top_level_get_request_reports_schemeful_lax_for_cross_scheme_same_site_initiator_mirrored() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("http://example.com/foo/bar");
    let initiator_url = parse("https://example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::top_level_navigation("GET")
            .with_initiator_url(&request_url, &initiator_url),
    );
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLax
    );
}

#[test]
fn top_level_post_request_reports_schemeful_lax_unsafe_for_cross_scheme_same_site_initiator() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://example.com/foo/bar");
    let initiator_url = parse("http://example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/foo; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::top_level_navigation("POST")
            .with_initiator_url(&request_url, &initiator_url),
    );
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert_eq!(
        lax.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe
    );
}

#[test]
fn top_level_post_request_reports_schemeful_lax_unsafe_for_cross_scheme_same_site_initiator_mirrored()
 {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("http://example.com/foo/bar");
    let initiator_url = parse("https://example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/foo; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::top_level_navigation("POST")
            .with_initiator_url(&request_url, &initiator_url),
    );
    let lax = find_cookie(&report, "lax").expect("lax cookie should be present");

    assert_eq!(
        lax.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe
    );
}

#[test]
fn subresource_request_reports_schemeful_cross_site_for_cross_scheme_same_site_initiator() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://example.com/foo/bar");
    let initiator_url = parse("http://example.com/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET")
            .with_initiator_url(&request_url, &initiator_url),
    );
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn subresource_request_reports_schemeful_cross_site_for_cross_scheme_site_for_cookies() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://example.com/foo/bar");
    let site_for_cookies_url = parse("http://example.com/frame.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET")
            .with_site_for_cookies_url(&request_url, &site_for_cookies_url)
            .with_top_frame_origin_url(&request_url, &site_for_cookies_url),
    );
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.site_for_cookies_url.as_ref(),
        Some(&site_for_cookies_url)
    );
    assert_eq!(
        strict.top_frame_origin_url.as_ref(),
        Some(&site_for_cookies_url)
    );
}

#[test]
fn subresource_post_reports_schemeful_cross_site_for_cross_scheme_site_for_cookies() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://example.com/foo/bar");
    let site_for_cookies_url = parse("http://example.com/frame.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("POST")
            .with_site_for_cookies_url(&request_url, &site_for_cookies_url)
            .with_top_frame_origin_url(&request_url, &site_for_cookies_url),
    );
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::SameSiteStrict
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        strict.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
}

#[test]
fn subresource_get_with_cross_site_initiator_stays_cross_site_even_if_site_for_cookies_is_only_cross_scheme()
 {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://example.com/foo/bar");
    let site_for_cookies_url = parse("http://example.com/frame.html");
    let cross_site_initiator_url = parse("https://cross-site.test/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET")
            .with_site_for_cookies_url(&request_url, &site_for_cookies_url)
            .with_top_frame_origin_url(&request_url, &site_for_cookies_url)
            .with_initiator_url(&request_url, &cross_site_initiator_url),
    );
    let strict = find_cookie(&report, "strict")
        .expect("strict cookie should still be represented in report");

    assert_eq!(
        strict.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteStrict]
    );
    assert_eq!(
        strict.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        strict.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
}

#[test]
fn subresource_post_with_cross_site_initiator_stays_cross_site_even_if_site_for_cookies_is_only_cross_scheme()
 {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://example.com/foo/bar");
    let site_for_cookies_url = parse("http://example.com/frame.html");
    let cross_site_initiator_url = parse("https://cross-site.test/index.html");

    store.store_response_headers(
        &request_url,
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/foo; Secure; SameSite=Lax".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("POST")
            .with_site_for_cookies_url(&request_url, &site_for_cookies_url)
            .with_top_frame_origin_url(&request_url, &site_for_cookies_url)
            .with_initiator_url(&request_url, &cross_site_initiator_url),
    );
    let lax =
        find_cookie(&report, "lax").expect("lax cookie should still be represented in report");

    assert_eq!(
        lax.exclusion_reasons,
        vec![StoredCookieExclusionReason::SameSiteLax]
    );
    assert_eq!(
        lax.same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.schemeful_same_site_context,
        StoredCookieRequestSameSiteContext::CrossSite
    );
    assert_eq!(
        lax.same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
    assert_eq!(
        lax.schemeful_same_site_context_http_method,
        StoredCookieSameSiteHttpMethod::Post
    );
}

#[test]
fn request_access_report_projects_browser_site_context_snapshot() {
    let mut store = BrowserCookieStore::default();
    let request_url = parse("https://other.test/app/panel");
    let response_url = parse("https://other.test/app/index.html");
    let site_for_cookies_url = parse("https://top.example/frame.html");
    let top_frame_origin_url = parse("https://top.example/root");

    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/app; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::top_level_navigation("GET")
            .with_site_for_cookies_url(&request_url, &site_for_cookies_url)
            .with_top_frame_origin_url(&request_url, &top_frame_origin_url)
            .with_storage_access_status(NetworkStorageAccessStatus::Granted),
    );
    let strict = find_cookie(&report, "strict").expect("strict cookie should be present");

    assert_eq!(
        strict.site_for_cookies_url.as_ref(),
        Some(&site_for_cookies_url)
    );
    assert_eq!(
        strict.top_frame_origin_url.as_ref(),
        Some(&top_frame_origin_url)
    );
    assert_eq!(
        strict.storage_access_status,
        StoredCookieStorageAccessStatus::Granted
    );
    assert_eq!(
        strict.site_context_basis,
        StoredCookieSiteContextBasis::SiteForCookies
    );
}

#[test]
fn request_access_report_is_agnostic_to_source_port() {
    let mut store = BrowserCookieStore::default();
    let response_url = parse("https://example.com:8443/app/index.html");
    let request_url = parse("https://example.com:9443/app/panel");

    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Path=/app; Secure".to_owned(),
        )],
    );

    // RFC 6265 matching is port-agnostic: a cookie set on one port of a host
    // is attached to requests on every other port of the same host.
    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET"),
    );
    let sid = find_cookie(&report, "sid").expect("cookie should be present");
    assert_eq!(sid.exclusion_reasons, vec![]);
    assert!(
        report
            .included_cookies
            .iter()
            .any(|entry| entry.cookie.name == "sid")
    );
}

#[test]
fn host_only_cookie_is_shared_across_ports_of_the_same_host() {
    let mut store = BrowserCookieStore::default();
    let set_url = parse("http://127.0.0.1:8771/");
    let other_port_url = parse("http://127.0.0.1:8768/check");

    store.store_response_headers(
        &set_url,
        &[("set-cookie".to_owned(), "a=1; Path=/".to_owned())],
    );

    assert_eq!(store.document_cookie(&other_port_url), "a=1");
    assert_eq!(store.cookie_header(&other_port_url).as_deref(), Some("a=1"));
}

#[test]
fn request_access_report_projects_source_scheme_mismatch_exclusion() {
    let mut store = BrowserCookieStore::default();
    let response_url = parse("https://example.com:8443/app/index.html");
    let request_url = parse("http://example.com:8443/app/panel");

    store.store_response_headers(
        &response_url,
        &[("set-cookie".to_owned(), "sid=1; Path=/app".to_owned())],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET"),
    );
    let sid = find_cookie(&report, "sid").expect("cookie should be present");
    assert_eq!(
        sid.exclusion_reasons,
        vec![StoredCookieExclusionReason::SchemeMismatch]
    );
}

#[test]
fn request_access_report_accumulates_multiple_exclusion_reasons() {
    let mut store = BrowserCookieStore::default();
    let response_url = parse("https://example.com:8443/foo/bar");
    let request_url = parse("http://example.com:9443/bar");

    store.store_response_headers(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
        )],
    );

    let report = store.cookie_access_report_for_request(
        &request_url,
        NetworkCookieRequestContext::subresource("GET")
            .with_site_context(NetworkSiteContext::cross_site()),
    );
    let strict = find_cookie(&report, "strict").expect("cookie should be present");
    assert_eq!(
        strict.exclusion_reasons,
        vec![
            StoredCookieExclusionReason::PathMismatch,
            StoredCookieExclusionReason::SecureOnly,
            StoredCookieExclusionReason::SchemeMismatch,
            StoredCookieExclusionReason::SameSiteStrict,
        ]
    );
}

#[test]
fn request_access_report_projects_access_semantics_and_secure_access_capability() {
    let mut store = BrowserCookieStore::default();
    let secure_url = parse("https://example.com/foo/bar");
    let insecure_url = parse("http://example.com/foo/bar");

    store.store_response_headers(
        &secure_url,
        &[
            (
                "set-cookie".to_owned(),
                "strict=1; Path=/foo; Secure; SameSite=Strict".to_owned(),
            ),
            (
                "set-cookie".to_owned(),
                "sid=1; Path=/foo; Secure".to_owned(),
            ),
        ],
    );

    let secure_report = store.cookie_access_report_for_request(
        &secure_url,
        NetworkCookieRequestContext::subresource("GET"),
    );
    let strict = find_cookie(&secure_report, "strict").expect("strict cookie should be present");
    let sid = find_cookie(&secure_report, "sid").expect("sid cookie should be present");

    assert_eq!(
        strict.access_semantics,
        StoredCookieAccessSemantics::NonLegacy
    );
    assert_eq!(strict.scope_semantics, StoredCookieScopeSemantics::Unknown);
    assert!(strict.is_allowed_to_access_secure_cookies);

    assert_eq!(sid.access_semantics, StoredCookieAccessSemantics::Unknown);
    assert!(sid.is_allowed_to_access_secure_cookies);

    let insecure_report = store.cookie_access_report_for_request(
        &insecure_url,
        NetworkCookieRequestContext::subresource("GET"),
    );
    let strict_insecure =
        find_cookie(&insecure_report, "strict").expect("strict cookie should remain observable");

    assert!(
        insecure_report
            .excluded_cookies
            .iter()
            .any(|entry| entry.cookie.name == "strict")
    );
    assert_eq!(
        strict_insecure.exclusion_reasons,
        vec![
            StoredCookieExclusionReason::SecureOnly,
            StoredCookieExclusionReason::SchemeMismatch,
        ]
    );
    assert!(!strict_insecure.is_allowed_to_access_secure_cookies);
}

#[test]
fn cookie_replacement_preserves_creation_order() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/");

    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "first=1; Path=/".to_owned())],
    );
    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "second=2; Path=/".to_owned())],
    );
    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "first=3; Path=/".to_owned())],
    );

    assert_eq!(
        store.cookie_header(&url),
        Some("first=3; second=2".to_owned())
    );
}

#[test]
fn cdp_upsert_applies_prefix_and_same_site_guards() {
    let mut store = BrowserCookieStore::default();

    store.upsert(
        StoredCookie {
            name: "__Host-bad".to_owned(),
            value: "1".to_owned(),
            domain: "example.com".to_owned(),
            host_only: false,
            path: "/".to_owned(),
            secure: true,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::Unspecified,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::Unset,
            source_port: -1,
            creation_index: 999,
            last_access_index: 999,
        },
        CookieSource::Cdp,
    );
    store.upsert(
        StoredCookie {
            name: "cross".to_owned(),
            value: "1".to_owned(),
            domain: "example.com".to_owned(),
            host_only: true,
            path: "/".to_owned(),
            secure: false,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::None,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::Unset,
            source_port: -1,
            creation_index: 999,
            last_access_index: 999,
        },
        CookieSource::Cdp,
    );
    store.upsert(
        StoredCookie {
            name: "__Host-good".to_owned(),
            value: "1".to_owned(),
            domain: "example.com".to_owned(),
            host_only: true,
            path: "/".to_owned(),
            secure: true,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::Unspecified,
            priority: None,
            partition_key: None,
            source_scheme: StoredCookieSourceScheme::Unset,
            source_port: -1,
            creation_index: 999,
            last_access_index: 999,
        },
        CookieSource::Cdp,
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("__Host-good=1".to_owned())
    );
}

#[test]
fn partitioned_cookie_is_stored_from_http_and_document_sources() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "__Host-chip=1; Secure; Path=/; Partitioned".to_owned(),
        )],
    );
    store.set_document_cookie(&url, "__Host-domchip=1; Secure; Path=/; Partitioned");

    assert_eq!(
        store.cookie_header(&url),
        Some("__Host-chip=1; __Host-domchip=1".to_owned())
    );
}

#[test]
fn cdp_upsert_accepts_partitioned_cookie() {
    let mut store = BrowserCookieStore::default();

    store.upsert(
        StoredCookie {
            name: "__Host-chip".to_owned(),
            value: "1".to_owned(),
            domain: "example.com".to_owned(),
            host_only: true,
            path: "/".to_owned(),
            secure: true,
            http_only: false,
            expires: None,
            same_site: StoredCookieSameSite::None,
            priority: None,
            partition_key: Some(StoredCookiePartitionKey::site(
                "https://example.com".to_owned(),
                false,
            )),
            source_scheme: StoredCookieSourceScheme::Secure,
            source_port: 443,
            creation_index: 999,
            last_access_index: 999,
        },
        CookieSource::Cdp,
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("__Host-chip=1".to_owned())
    );
}

#[test]
fn third_party_response_cookie_is_scoped_to_top_level_site() {
    let mut store = BrowserCookieStore::default();
    let response_url = parse("https://challenges.cloudflare.com/turnstile/v0/");
    let top_a = parse("https://resetera.com/thread");
    let top_b = parse("https://other.example/page");
    let context_a =
        NetworkCookieRequestContext::subresource("POST").with_initiator_url(&response_url, &top_a);
    let reports = store.store_response_headers_with_context_reports(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "cf_clearance=token; Secure; SameSite=None; Partitioned; Path=/".to_owned(),
        )],
        &context_a,
    );

    assert_eq!(reports.len(), 1);
    assert!(reports[0].is_accepted());
    assert_eq!(
        store.cookie_header_for_request(&response_url, context_a),
        Some("cf_clearance=token".to_owned())
    );
    let context_b =
        NetworkCookieRequestContext::subresource("POST").with_initiator_url(&response_url, &top_b);
    assert_eq!(
        store.cookie_header_for_request(&response_url, context_b),
        None
    );

    let cookie = store
        .cookies()
        .into_iter()
        .find(|cookie| cookie.name == "cf_clearance")
        .expect("partitioned clearance should remain stored");
    assert_eq!(
        cookie.partition_key,
        Some(StoredCookiePartitionKey::site(
            "https://resetera.com".to_owned(),
            false,
        ))
    );
}

#[test]
fn delete_cookies_with_partition_key_keeps_other_top_level_sites() {
    let mut store = BrowserCookieStore::default();
    let response_url = parse("https://widget.example/resource");
    let top_a = parse("https://first.example/page");
    let top_b = parse("https://second.example/page");
    let context_a =
        NetworkCookieRequestContext::subresource("GET").with_initiator_url(&response_url, &top_a);
    let context_b =
        NetworkCookieRequestContext::subresource("GET").with_initiator_url(&response_url, &top_b);
    for (value, context) in [("one", &context_a), ("two", &context_b)] {
        store.store_response_headers_with_context_reports(
            &response_url,
            &[(
                "set-cookie".to_owned(),
                format!("chip={value}; Secure; SameSite=None; Partitioned; Path=/"),
            )],
            context,
        );
    }

    let removed = store.delete_cookies_with_partition_key(
        Some("chip"),
        Some("widget.example"),
        Some("/"),
        None,
        Some(&StoredCookiePartitionKey::site(
            "https://first.example".to_owned(),
            false,
        )),
    );

    assert_eq!(removed, 1);
    assert_eq!(
        store.cookie_header_for_request(&response_url, context_a),
        None
    );
    assert_eq!(
        store.cookie_header_for_request(&response_url, context_b),
        Some("chip=two".to_owned())
    );
}

#[test]
fn response_cookie_records_priority_and_source_metadata() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com:8443/"),
        &[(
            "set-cookie".to_owned(),
            "prio=1; Path=/; Secure; Priority=High".to_owned(),
        )],
    );

    let cookie = store
        .cookies()
        .into_iter()
        .find(|cookie| cookie.name == "prio")
        .expect("cookie should be stored");
    assert_eq!(cookie.priority, Some(CookiePriority::High));
    assert_eq!(cookie.source_scheme, StoredCookieSourceScheme::Secure);
    assert_eq!(cookie.source_port, 8443);
}

#[test]
fn max_age_takes_precedence_over_expires_for_immediate_removal() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/");

    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "sid=1; Path=/".to_owned())],
    );
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=2; Path=/; Max-Age=0; Expires=Wed, 21 Oct 2099 07:28:00 GMT".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), None);
}

#[test]
fn max_age_takes_precedence_over_past_expires_when_positive() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Path=/; Max-Age=3600; Expires=Wed, 21 Oct 2015 07:28:00 GMT".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&url), Some("sid=1".to_owned()));
}

#[test]
fn oversized_path_attribute_is_ignored_and_defaults_request_path() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/nested/index.html");
    let oversized_path = format!("/{}", "a".repeat(1024));

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            format!("fallback=1; Path={oversized_path}"),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/nested/child")),
        Some("fallback=1".to_owned())
    );
    assert_eq!(store.cookie_header(&parse("https://example.com/")), None);
}

#[test]
fn oversized_domain_attribute_is_ignored_and_cookie_becomes_host_only() {
    let mut store = BrowserCookieStore::default();
    let url = parse("https://example.com/");
    let oversized_domain = format!("{}.com", "a".repeat(1021));

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            format!("hostonly=1; Domain={oversized_domain}; Path=/"),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("hostonly=1".to_owned())
    );
    assert_eq!(
        store.cookie_header(&parse("https://sub.example.com/")),
        None
    );
}

#[test]
fn oversized_cookie_name_plus_value_is_rejected() {
    let mut store = BrowserCookieStore::default();
    let oversized_value = "a".repeat(4097);

    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            format!("huge={oversized_value}; Path=/; Secure"),
        )],
    );

    assert_eq!(store.cookie_header(&parse("https://example.com/")), None);
}

#[test]
fn per_domain_eviction_prefers_removing_non_secure_cookie() {
    let mut store = BrowserCookieStore::new_with_limits(5, 100);
    let url = parse("https://example.com/");

    for index in 1..=4 {
        store.store_response_headers(
            &url,
            &[(
                "set-cookie".to_owned(),
                format!("secure{index}=1; Path=/; Secure"),
            )],
        );
    }
    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "plain=1; Path=/".to_owned())],
    );
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "newsecure=1; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("secure1=1; secure2=1; secure3=1; secure4=1; newsecure=1".to_owned())
    );
}

#[test]
fn per_domain_eviction_rejects_new_non_secure_cookie_when_all_existing_are_secure() {
    let mut store = BrowserCookieStore::new_with_limits(5, 100);
    let url = parse("https://example.com/");

    for index in 1..=5 {
        store.store_response_headers(
            &url,
            &[(
                "set-cookie".to_owned(),
                format!("secure{index}=1; Path=/; Secure"),
            )],
        );
    }
    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "plain=1; Path=/".to_owned())],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("secure1=1; secure2=1; secure3=1; secure4=1; secure5=1".to_owned())
    );
}

#[test]
fn per_domain_eviction_removes_oldest_non_secure_cookie_first() {
    let mut store = BrowserCookieStore::new_with_limits(5, 100);
    let url = parse("https://example.com/");

    for index in 1..=5 {
        store.store_response_headers(
            &url,
            &[("set-cookie".to_owned(), format!("plain{index}=1; Path=/"))],
        );
    }
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "secure=1; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("plain2=1; plain3=1; plain4=1; plain5=1; secure=1".to_owned())
    );
}

#[test]
fn expired_cookies_are_removed_before_domain_eviction() {
    let mut store = BrowserCookieStore::new_with_limits(5, 100);
    let url = parse("https://example.com/");

    for index in 1..=5 {
        store.store_response_headers(
            &url,
            &[(
                "set-cookie".to_owned(),
                format!("stale{index}=1; Path=/; Expires=Wed, 21 Oct 2015 07:28:00 GMT"),
            )],
        );
    }
    store.store_response_headers(
        &url,
        &[("set-cookie".to_owned(), "fresh=1; Path=/".to_owned())],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("fresh=1".to_owned())
    );
}

#[test]
fn global_eviction_applies_when_total_cookie_limit_is_hit() {
    let mut store = BrowserCookieStore::new_with_limits(10, 3);

    store.store_response_headers(
        &parse("https://one.example/"),
        &[("set-cookie".to_owned(), "a=1; Path=/; Secure".to_owned())],
    );
    store.store_response_headers(
        &parse("https://two.example/"),
        &[("set-cookie".to_owned(), "b=1; Path=/; Secure".to_owned())],
    );
    store.store_response_headers(
        &parse("https://three.example/"),
        &[("set-cookie".to_owned(), "c=1; Path=/".to_owned())],
    );
    store.store_response_headers(
        &parse("https://four.example/"),
        &[("set-cookie".to_owned(), "d=1; Path=/; Secure".to_owned())],
    );

    assert_eq!(store.cookies().len(), 3);
    assert_eq!(store.cookie_header(&parse("https://three.example/")), None);
    assert_eq!(
        store.cookie_header(&parse("https://four.example/")),
        Some("d=1".to_owned())
    );
}

#[test]
fn eviction_prefers_lower_priority_before_higher_priority() {
    let mut store = BrowserCookieStore::new_with_limits(3, 100);
    let url = parse("https://example.com/");

    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "low=1; Path=/; Priority=Low".to_owned(),
        )],
    );
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "high=1; Path=/; Priority=High".to_owned(),
        )],
    );
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "medium=1; Path=/; Priority=Medium".to_owned(),
        )],
    );
    store.store_response_headers(
        &url,
        &[(
            "set-cookie".to_owned(),
            "high2=1; Path=/; Priority=High".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&url),
        Some("high=1; medium=1; high2=1".to_owned())
    );
}

#[test]
fn invalid_priority_falls_back_to_medium() {
    let mut store = BrowserCookieStore::default();

    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "prio=1; Path=/; Priority=NotReal".to_owned(),
        )],
    );

    let cookie = store
        .cookies()
        .into_iter()
        .find(|cookie| cookie.name == "prio")
        .expect("cookie should be stored");
    assert_eq!(cookie.priority, None);
    assert_eq!(cookie.effective_priority(), CookiePriority::Medium);
}

#[test]
fn upstream_samesite_request_context_matrix() {
    let mut store = BrowserCookieStore::default();
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; SameSite=Strict; Secure".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "lax=1; Path=/; SameSite=Lax; Secure".to_owned(),
        )],
    );
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "none=1; Path=/; SameSite=None; Secure".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/resource")),
        Some("strict=1; lax=1; none=1".to_owned())
    );
}

#[test]
fn upstream_partitioned_cookie_cases() {
    let mut store = BrowserCookieStore::default();
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "chip=1; Path=/; Secure; SameSite=None; Partitioned".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://example.com/")),
        Some("chip=1".to_owned())
    );
}

#[test]
fn upstream_cookie_quota_and_eviction_cases() {
    let mut store = BrowserCookieStore::default();

    for index in 0..200 {
        store.store_response_headers(
            &parse("https://example.com/"),
            &[(
                "set-cookie".to_owned(),
                format!("c{index}=1; Path=/; Secure"),
            )],
        );
    }

    assert!(store.cookies().len() <= 180);
}

#[test]
fn upstream_schemeful_samesite_treats_http_to_https_as_cross_site() {
    let mut store = BrowserCookieStore::default();
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "strict=1; Path=/; SameSite=Strict; Secure".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&parse("http://example.com/")), None);
}

#[test]
fn upstream_partitioned_cookie_isolated_by_top_level_site() {
    let mut store = BrowserCookieStore::default();
    let widget_url = parse("https://widget.example/");
    let top_level = parse("https://top.example/");
    let matching_context =
        NetworkCookieRequestContext::subresource("GET").with_initiator_url(&widget_url, &top_level);
    store.store_response_headers_with_context_reports(
        &widget_url,
        &[(
            "set-cookie".to_owned(),
            "chip=1; Path=/; Secure; SameSite=None; Partitioned".to_owned(),
        )],
        &matching_context,
    );

    assert_eq!(
        store.cookie_header_for_request(
            &parse("https://widget.example/resource"),
            NetworkCookieRequestContext::subresource("GET")
                .with_initiator_url(&widget_url, &parse("https://other.example/")),
        ),
        None
    );
    assert_eq!(
        store.cookie_header_for_request(
            &parse("https://widget.example/resource"),
            matching_context,
        ),
        Some("chip=1".to_owned())
    );
}

#[test]
fn upstream_public_suffix_domain_rejection() {
    let mut store = BrowserCookieStore::default();
    store.store_response_headers(
        &parse("https://foo.co.uk/"),
        &[(
            "set-cookie".to_owned(),
            "wide=1; Domain=co.uk; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(store.cookie_header(&parse("https://bar.co.uk/")), None);
}

#[test]
fn deeper_rule_does_not_shadow_parent_wildcard_during_domain_rejection() {
    let mut store = BrowserCookieStore::default();
    store.store_response_headers(
        &parse("https://foo.oci.customer-oci.com/"),
        &[(
            "set-cookie".to_owned(),
            "wide=1; Domain=oci.customer-oci.com; Path=/; Secure".to_owned(),
        )],
    );

    assert!(store.cookies().is_empty());
}

#[test]
fn public_suffix_identical_host_domain_downgrades_to_host_only() {
    let mut store = BrowserCookieStore::default();
    store.store_response_headers(
        &parse("https://github.io/"),
        &[(
            "set-cookie".to_owned(),
            "hostonly=1; Domain=github.io; Path=/; Secure".to_owned(),
        )],
    );

    assert_eq!(
        store.cookie_header(&parse("https://github.io/")),
        Some("hostonly=1".to_owned())
    );
    assert_eq!(store.cookie_header(&parse("https://foo.github.io/")), None);
}

#[test]
fn upstream_priority_influences_eviction_order() {
    let mut store = BrowserCookieStore::default();

    for index in 0..50 {
        store.store_response_headers(
            &parse("https://example.com/"),
            &[(
                "set-cookie".to_owned(),
                format!("low{index}=1; Path=/; Secure; Priority=Low"),
            )],
        );
    }
    store.store_response_headers(
        &parse("https://example.com/"),
        &[(
            "set-cookie".to_owned(),
            "high=1; Path=/; Secure; Priority=High".to_owned(),
        )],
    );

    assert!(
        store
            .document_cookie(&parse("https://example.com/"))
            .contains("high=1")
    );
}

#[test]
fn upstream_cookie_source_metadata_is_preserved() {
    let mut store = BrowserCookieStore::default();
    store.store_response_headers(
        &parse("https://example.com:8443/"),
        &[("set-cookie".to_owned(), "sid=1; Path=/; Secure".to_owned())],
    );

    let cookie = store
        .cookies()
        .into_iter()
        .find(|cookie| cookie.name == "sid");
    assert!(cookie.is_some());
}
