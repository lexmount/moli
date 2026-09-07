//! Browser cookie store wrapper and cross-crate entry points.

mod site_data;

use std::borrow::Cow;
use std::sync::Arc;

use cookie_store::{
    CanonicalCookieInput, CookieDeleteFilter, CookieExpiration, CookieStore, CookieStoreLimits,
    HttpRequestType, InsertContext, QueryContext, SameSite, SameSiteContext,
    SameSiteRequestContext,
};
use moli_site::{public_suffix_list, same_site_urls};
use parking_lot::Mutex;
use url::Url;

use super::model::{
    BrowserCookieFacadeContext, CookieSource, NetworkCookieRequestContext,
    NetworkCookieRequestType, NetworkSameSiteContext, NetworkSameSiteHttpMethod,
    NetworkSameSiteRedirectType, StoredCookie, StoredCookieQueryReport, StoredCookieSameSite,
    StoredCookieSetRejectionReason, StoredCookieSetReport, StoredCookieSetStatus,
    StoredCookieSourceScheme, core_browser_site_context_from_facade,
    core_cookie_partition_key_for_url, core_partition_key_from_stored,
    core_same_site_context_metadata_from_stored, core_source_scheme_from_stored,
    has_invalid_cookie_octets, stored_query_report_from_core, stored_set_report_from_core,
};

/// Thread-shareable cookie store used by browser/runtime components.
pub type SharedBrowserCookieStore = Arc<Mutex<BrowserCookieStore>>;

/// Builds a shared cookie store with browser-grade defaults.
pub fn new_shared_browser_cookie_store() -> SharedBrowserCookieStore {
    Arc::new(Mutex::new(BrowserCookieStore::default()))
}

const DEFAULT_PER_DOMAIN_MAX_COOKIES: usize = 180;
const DEFAULT_GLOBAL_MAX_COOKIES: usize = 3300;

/// Browser-facing wrapper around the canonical cookie store.
///
/// This type owns policy, diagnostics, and DTO conversion expected by
/// Moli callers while delegating canonical parsing, matching, quota, and
/// eviction to the forked `cookie_store` implementation.
#[derive(Debug, Clone)]
pub struct BrowserCookieStore {
    // Canonical matching core for all accepted writes. Browser-specific policy now projects
    // directly from this store so Moli does not keep a second payload mirror.
    pub(super) full_core: CookieStore,
    // Shared browser-context state must not own per-document JS visibility
    // policy or cache entries. Instead it publishes a coarse generation that
    // document-bound facades can use to invalidate their own cache when the
    // canonical cookie set changes.
    document_cookie_generation: u64,
}

impl Default for BrowserCookieStore {
    fn default() -> Self {
        Self::new_with_limits(DEFAULT_PER_DOMAIN_MAX_COOKIES, DEFAULT_GLOBAL_MAX_COOKIES)
    }
}

impl BrowserCookieStore {
    /// Builds a store with explicit per-domain and global cookie limits.
    pub fn new_with_limits(per_domain_max_cookies: usize, global_max_cookies: usize) -> Self {
        let limits = CookieStoreLimits::new(per_domain_max_cookies, global_max_cookies);
        Self {
            // Quota/eviction now lives in the fork core because it depends on
            // canonical priority and access metadata owned there.
            // The fork only enforces public-suffix rejection when it owns an
            // actual PSL instance. Wire the vendored Moli snapshot into
            // the default jar so browser-grade Domain rejection is enabled in
            // real code paths instead of only behind explicit test setup.
            full_core: CookieStore::default()
                .with_shared_suffix_list(public_suffix_list())
                .with_limits(limits),
            document_cookie_generation: 0,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn cookie_header(&mut self, url: &Url) -> Option<String> {
        self.cookie_header_for_request(url, NetworkCookieRequestContext::subresource("GET"))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn cookie_header_for_request(
        &mut self,
        url: &Url,
        request_context: NetworkCookieRequestContext,
    ) -> Option<String> {
        self.purge_expired();
        let values = self.full_core.get_ordered_request_values_with_context(
            &network_query_context(url, request_context)
                .with_return_excluded_cookies(false)
                .with_update_access_time(true),
        );
        if values.is_empty() {
            return None;
        }
        Some(
            values
                .into_iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn document_cookie(&mut self, url: &Url) -> String {
        self.document_cookie_with_context(url, &BrowserCookieFacadeContext::default())
    }

    /// Returns the `document.cookie` string visible under the given facade context.
    pub fn document_cookie_with_context(
        &mut self,
        url: &Url,
        browser_context: &BrowserCookieFacadeContext,
    ) -> String {
        self.purge_expired();
        let effective_url = effective_document_cookie_url(url);
        self.full_core
            .get_ordered_request_values_with_context(
                &document_query_context(effective_url.as_ref(), browser_context)
                    .with_return_excluded_cookies(false)
                    .with_update_access_time(true),
            )
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_document_cookie(&mut self, document_url: &Url, cookie: &str) {
        let _ = self.set_document_cookie_with_report(document_url, cookie);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_document_cookie_with_report(
        &mut self,
        document_url: &Url,
        cookie: &str,
    ) -> StoredCookieSetReport {
        self.set_document_cookie_with_context_report(
            document_url,
            cookie,
            &BrowserCookieFacadeContext::default(),
        )
    }

    /// Applies a `document.cookie` write and returns browser-policy diagnostics.
    pub fn set_document_cookie_with_context_report(
        &mut self,
        document_url: &Url,
        cookie: &str,
        browser_context: &BrowserCookieFacadeContext,
    ) -> StoredCookieSetReport {
        if has_invalid_cookie_octets(cookie) {
            return StoredCookieSetReport {
                status: StoredCookieSetStatus::Rejected(
                    StoredCookieSetRejectionReason::InvalidOctets,
                ),
                rejection_reasons: vec![StoredCookieSetRejectionReason::InvalidOctets],
                warning_reasons: Vec::new(),
                effective_same_site: None,
            };
        }

        let normalized = match cookie.split_once(';') {
            Some((first, rest)) if !first.contains('=') => format!("{first}=;{rest}"),
            None if !cookie.contains('=') => format!("{cookie}="),
            _ => cookie.to_owned(),
        };
        let normalized = normalize_expires_utc_timezone(&normalized);
        let effective_document_url = effective_document_cookie_url(document_url);
        let report =
            stored_set_report_from_core(self.full_core.set_response_cookie_str_with_access_result(
                normalized.as_ref(),
                &document_insert_context(effective_document_url.as_ref(), browser_context),
            ));
        if report.is_accepted() {
            self.bump_document_cookie_generation();
        }
        report
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn store_response_headers(&mut self, response_url: &Url, headers: &[(String, String)]) {
        let reports = self.store_response_headers_with_reports(response_url, headers);
        for ((_, value), report) in headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            .zip(reports.iter())
        {
            if !report.is_accepted() || !report.warning_reasons.is_empty() {
                tracing::debug!(?report, header = %value, "processed set-cookie header");
            }
        }
    }

    /// Stores all `Set-Cookie` response headers and reports each write outcome.
    pub fn store_response_headers_with_reports(
        &mut self,
        response_url: &Url,
        headers: &[(String, String)],
    ) -> Vec<StoredCookieSetReport> {
        self.store_response_headers_with_context_reports(
            response_url,
            headers,
            &NetworkCookieRequestContext::top_level_navigation("GET"),
        )
    }

    /// Stores response cookies using the exact request-side browser partition
    /// context that produced the response.
    pub fn store_response_headers_with_context_reports(
        &mut self,
        response_url: &Url,
        headers: &[(String, String)],
        request_context: &NetworkCookieRequestContext,
    ) -> Vec<StoredCookieSetReport> {
        let mut reports = Vec::new();
        let insert_context = network_insert_context(response_url, request_context);
        for (_, value) in headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
        {
            let normalized = normalize_expires_utc_timezone(value);
            reports.push(stored_set_report_from_core(
                self.full_core.set_response_cookie_str_with_access_result(
                    normalized.as_ref(),
                    &insert_context,
                ),
            ));
        }
        if reports.iter().any(StoredCookieSetReport::is_accepted) {
            self.bump_document_cookie_generation();
        }
        reports
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn store_cookie(
        &mut self,
        response_url: &Url,
        cookie: cookie_store::Cookie<'static>,
        source: CookieSource,
        priority: cookie_store::CookiePriority,
    ) -> bool {
        self.purge_expired();

        // Expiry-driven removals and ordinary inserts both flow through the
        // same structured core set path now. The wrapper deliberately does not
        // re-classify them, so accepted/rejected semantics stay owned by the
        // fork and this boundary only projects them down to `bool`.
        let mut cookie = cookie;
        cookie.set_priority(priority);
        matches!(
            self.full_core
                .set_with_context(cookie, &insert_context(response_url, source)),
            cookie_store::CookieSetResult::Accepted(_)
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_last_access_index(&self, domain: &str, path: &str, name: &str) -> Option<u64> {
        self.full_core
            .get(domain, path, name)
            .map(|cookie| cookie.last_access_index())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn upsert(&mut self, cookie: StoredCookie, source: CookieSource) {
        let _ = self.upsert_with_request_url_report(cookie, None, source);
    }

    /// Inserts or replaces a structured cookie DTO, usually from CDP/profile APIs.
    pub fn upsert_with_request_url_report(
        &mut self,
        cookie: StoredCookie,
        request_url: Option<&Url>,
        source: CookieSource,
    ) -> StoredCookieSetReport {
        self.purge_expired();

        if !cookie.is_expired() {
            if let Some((core_cookie, request_url)) =
                canonical_cookie_input_from_stored_cookie(&cookie, request_url)
            {
                let report = stored_set_report_from_core(
                    self.full_core.set_canonical_cookie_with_access_result(
                        core_cookie,
                        &insert_context(&request_url, source),
                    ),
                );
                if report.is_accepted() {
                    self.bump_document_cookie_generation();
                }
                return report;
            }
        } else {
            self.full_core.remove_with_partition_key(
                &cookie.domain,
                &cookie.path,
                &cookie.name,
                cookie
                    .partition_key
                    .as_ref()
                    .map(core_partition_key_from_stored)
                    .as_ref(),
            );
            self.bump_document_cookie_generation();
            return StoredCookieSetReport {
                status: StoredCookieSetStatus::Accepted(cookie_store::StoreAction::ExpiredExisting),
                rejection_reasons: Vec::new(),
                warning_reasons: Vec::new(),
                effective_same_site: None,
            };
        }

        StoredCookieSetReport {
            status: StoredCookieSetStatus::Rejected(
                StoredCookieSetRejectionReason::NonRelativeScheme,
            ),
            rejection_reasons: vec![StoredCookieSetRejectionReason::NonRelativeScheme],
            warning_reasons: Vec::new(),
            effective_same_site: None,
        }
    }

    /// Removes expired cookies and advances the document-cookie generation.
    pub fn purge_expired(&mut self) {
        let expired = self
            .full_core
            .iter_any()
            .filter(|cookie| cookie.is_expired())
            .map(|cookie| {
                (
                    String::from(&cookie.domain),
                    String::from(&cookie.path),
                    cookie.name().to_owned(),
                    cookie.partition_key().cloned(),
                )
            })
            .collect::<Vec<_>>();
        if expired.is_empty() {
            return;
        }
        for (domain, path, name, partition_key) in expired {
            self.full_core
                .remove_with_partition_key(&domain, &path, &name, partition_key.as_ref());
        }
        self.bump_document_cookie_generation();
    }

    /// Returns all unexpired cookies projected into stable Moli DTOs.
    pub fn cookies(&mut self) -> Vec<StoredCookie> {
        self.purge_expired();
        let mut cookies = self
            .full_core
            .iter_unexpired()
            .map(super::model::stored_cookie_from_core)
            .collect::<Vec<_>>();
        cookies.sort_by_key(|cookie| cookie.creation_index);
        cookies
    }

    /// Deletes cookies matching optional CDP-style name/domain/path/url filters.
    pub fn delete_cookies(
        &mut self,
        name: Option<&str>,
        domain: Option<&str>,
        path: Option<&str>,
        url_host: Option<&str>,
    ) -> usize {
        self.delete_cookies_with_partition_key(name, domain, path, url_host, None)
    }

    /// Deletes cookies matching optional CDP filters and, when provided, one
    /// exact CHIPS partition key.
    pub fn delete_cookies_with_partition_key(
        &mut self,
        name: Option<&str>,
        domain: Option<&str>,
        path: Option<&str>,
        url_host: Option<&str>,
        partition_key: Option<&super::model::StoredCookiePartitionKey>,
    ) -> usize {
        self.purge_expired();
        let partition_key = partition_key.map(core_partition_key_from_stored);
        let removed = self.full_core.delete_matching(&CookieDeleteFilter {
            name,
            domain,
            path,
            url_host,
            partition_key: partition_key.as_ref(),
        });
        if removed > 0 {
            self.bump_document_cookie_generation();
        }
        removed
    }

    /// Produces request cookie diagnostics and updates last-access metadata.
    pub fn cookie_access_report_for_request(
        &mut self,
        url: &Url,
        request_context: NetworkCookieRequestContext,
    ) -> StoredCookieQueryReport {
        self.cookie_access_report_for_request_with_options(url, request_context, true)
    }

    /// Produces request cookie diagnostics without updating last-access metadata.
    pub fn observe_cookie_access_report_for_request(
        &mut self,
        url: &Url,
        request_context: NetworkCookieRequestContext,
    ) -> StoredCookieQueryReport {
        // Post-hoc observers such as CDP request events need the same
        // included/excluded diagnostics as the real request path, but they must
        // not count as a second cookie read for access-time bookkeeping.
        self.cookie_access_report_for_request_with_options(url, request_context, false)
    }

    fn cookie_access_report_for_request_with_options(
        &mut self,
        url: &Url,
        request_context: NetworkCookieRequestContext,
        update_access_time: bool,
    ) -> StoredCookieQueryReport {
        self.purge_expired();
        // The browser-facing boundary now projects the richer fork access
        // result into Moli-owned DTOs. That keeps diagnostics available
        // to browser layers without leaking fork types into JS/CDP/network.
        stored_query_report_from_core(
            self.full_core.get_ordered_cookie_access_query_result(
                &network_query_context(url, request_context)
                    .with_return_excluded_cookies(true)
                    .with_update_access_time(update_access_time),
            ),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn document_cookie_access_report(&mut self, url: &Url) -> StoredCookieQueryReport {
        self.document_cookie_access_report_with_options(
            url,
            &BrowserCookieFacadeContext::default(),
            true,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn document_cookie_access_report_with_context(
        &mut self,
        url: &Url,
        browser_context: &BrowserCookieFacadeContext,
    ) -> StoredCookieQueryReport {
        self.document_cookie_access_report_with_options(url, browser_context, true)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn document_cookie_access_report_with_options(
        &mut self,
        url: &Url,
        browser_context: &BrowserCookieFacadeContext,
        update_access_time: bool,
    ) -> StoredCookieQueryReport {
        self.purge_expired();
        let effective_url = effective_document_cookie_url(url);
        stored_query_report_from_core(
            self.full_core.get_ordered_cookie_access_query_result(
                &document_query_context(effective_url.as_ref(), browser_context)
                    .with_update_access_time(update_access_time),
            ),
        )
    }

    /// Monotonic generation for invalidating document-bound cookie caches.
    pub fn document_cookie_generation(&self) -> u64 {
        self.document_cookie_generation
    }

    fn bump_document_cookie_generation(&mut self) {
        self.document_cookie_generation = self.document_cookie_generation.wrapping_add(1);
    }
}

fn normalize_expires_utc_timezone(raw: &str) -> Cow<'_, str> {
    let mut changed = false;
    let segments = raw
        .split(';')
        .map(|segment| {
            let trimmed = segment.trim_start();
            let Some(eq_index) = trimmed.find('=') else {
                return segment.to_owned();
            };
            let (name, value_with_equals) = trimmed.split_at(eq_index);
            if !name.eq_ignore_ascii_case("expires") {
                return segment.to_owned();
            }
            let value = &value_with_equals[1..];
            let value_trimmed = value.trim_end();
            if value_trimmed.len() < 3
                || !value_trimmed[value_trimmed.len() - 3..].eq_ignore_ascii_case("UTC")
            {
                if !looks_like_http_date_without_timezone(value_trimmed) {
                    return segment.to_owned();
                }
                let leading = &segment[..segment.len() - trimmed.len()];
                let trailing = &value[value_trimmed.len()..];
                changed = true;
                return format!("{leading}{name}={value_trimmed} GMT{trailing}");
            }
            let leading = &segment[..segment.len() - trimmed.len()];
            let trailing = &value[value_trimmed.len()..];
            changed = true;
            format!(
                "{leading}{name}={}GMT{trailing}",
                &value_trimmed[..value_trimmed.len() - 3]
            )
        })
        .collect::<Vec<_>>();
    if changed {
        Cow::Owned(segments.join(";"))
    } else {
        Cow::Borrowed(raw)
    }
}

fn looks_like_http_date_without_timezone(value: &str) -> bool {
    let parts = value.split_ascii_whitespace().collect::<Vec<_>>();
    if parts.len() != 5 {
        return false;
    }
    let [weekday, day, month, year, time] = parts.as_slice() else {
        return false;
    };
    weekday.ends_with(',')
        && weekday[..weekday.len() - 1]
            .chars()
            .all(|ch| ch.is_ascii_alphabetic())
        && day.len() <= 2
        && day.chars().all(|ch| ch.is_ascii_digit())
        && month.len() == 3
        && month.chars().all(|ch| ch.is_ascii_alphabetic())
        && year.len() == 4
        && year.chars().all(|ch| ch.is_ascii_digit())
        && looks_like_http_time(time)
}

fn looks_like_http_time(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 3 {
        return false;
    }
    parts
        .iter()
        .all(|part| part.len() == 2 && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn canonical_cookie_input_from_stored_cookie(
    cookie: &StoredCookie,
    request_url_hint: Option<&Url>,
) -> Option<(CanonicalCookieInput, Url)> {
    // This path is only for metadata-originated writes such as CDP upserts. The wrapper now
    // converts browser-facing DTOs into a structured canonical-cookie input and lets the fork
    // handle raw-cookie assembly internally, instead of rebuilding `RawCookieBuilder` here.
    //
    // Keep the original request URL when the caller has one. Re-deriving it
    // from the cookie's canonical domain is wrong for structured writes like
    // CDP `Storage.setCookies`: a `Domain=co.uk` cookie coming from
    // `https://foo.co.uk/` must still be validated against `foo.co.uk`, not
    // silently "corrected" into `https://co.uk/`.
    let request_url = request_url_hint
        .cloned()
        .or_else(|| request_url_for_stored_cookie(cookie))?;
    Some((
        CanonicalCookieInput {
            name: cookie.name.clone(),
            value: cookie.value.clone(),
            domain: cookie.domain.clone(),
            host_only: cookie.host_only,
            path: cookie.path.clone(),
            secure: cookie.secure,
            http_only: cookie.http_only,
            same_site: match cookie.same_site {
                StoredCookieSameSite::Unspecified => None,
                StoredCookieSameSite::None => Some(SameSite::None),
                StoredCookieSameSite::Lax => Some(SameSite::Lax),
                StoredCookieSameSite::Strict => Some(SameSite::Strict),
            },
            expires: cookie
                .expires
                .map(CookieExpiration::AtUtc)
                .unwrap_or(CookieExpiration::SessionEnd),
            partition_key: cookie
                .partition_key
                .as_ref()
                .map(core_partition_key_from_stored),
            priority: cookie.priority,
            source_scheme: core_source_scheme_from_stored(cookie.source_scheme),
            source_port: cookie.source_port,
        },
        request_url,
    ))
}

fn request_url_for_stored_cookie(cookie: &StoredCookie) -> Option<Url> {
    let scheme = match cookie.source_scheme {
        // Structured browser-facing inputs already carry source-scheme
        // separately from the `Secure` attribute. Preserve that when
        // reconstructing the request URL so `http://localhost` Secure cookies
        // keep their trustworthy-non-cryptographic semantics instead of being
        // silently upgraded to `https://...`.
        StoredCookieSourceScheme::Secure => "https",
        StoredCookieSourceScheme::NonSecure => "http",
        StoredCookieSourceScheme::Unset => {
            if cookie.secure {
                "https"
            } else {
                "http"
            }
        }
    };
    let host = if cookie.domain.contains(':') && !cookie.domain.starts_with('[') {
        format!("[{}]", cookie.domain)
    } else {
        cookie.domain.clone()
    };
    Url::parse(&format!("{scheme}://{host}{}", cookie.path)).ok()
}

pub(super) fn network_query_context<'a>(
    url: &'a Url,
    request_context: NetworkCookieRequestContext,
) -> QueryContext<'a> {
    let mut context = QueryContext::http(url);
    context.same_site_context = SameSiteContext::new(
        match request_context.site_context.context {
            NetworkSameSiteContext::SameSiteStrict => SameSiteRequestContext::SameSiteStrict,
            NetworkSameSiteContext::SameSiteLax => SameSiteRequestContext::SameSiteLax,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe => {
                SameSiteRequestContext::SameSiteLaxMethodUnsafe
            }
            NetworkSameSiteContext::CrossSite => SameSiteRequestContext::CrossSite,
        },
        match request_context.site_context.schemeful_context {
            NetworkSameSiteContext::SameSiteStrict => SameSiteRequestContext::SameSiteStrict,
            NetworkSameSiteContext::SameSiteLax => SameSiteRequestContext::SameSiteLax,
            NetworkSameSiteContext::SameSiteLaxMethodUnsafe => {
                SameSiteRequestContext::SameSiteLaxMethodUnsafe
            }
            NetworkSameSiteContext::CrossSite => SameSiteRequestContext::CrossSite,
        },
    );
    context.browser_context =
        core_browser_site_context_from_facade(&request_context.browser_context);
    context.browser_context.cookie_partition_key = Some(core_cookie_partition_key_for_url(
        &request_context.browser_context,
        url,
    ));
    context.same_site_context_metadata =
        core_same_site_context_metadata_from_stored(request_context.site_context_metadata);
    // Method/redirect shape now rides inside the track metadata so richer
    // query diagnostics can stay attached to the same schemeless/schemeful
    // structure as downgrade information. Use the wrapper-provided request
    // shape directly instead of trying to reconstruct it from URL/source here.
    context.http_method = match request_context.site_context_metadata.context.http_method {
        NetworkSameSiteHttpMethod::Unset => cookie_store::SameSiteContextHttpMethod::Unset,
        NetworkSameSiteHttpMethod::Unknown => cookie_store::SameSiteContextHttpMethod::Unknown,
        NetworkSameSiteHttpMethod::Get => cookie_store::SameSiteContextHttpMethod::Get,
        NetworkSameSiteHttpMethod::Head => cookie_store::SameSiteContextHttpMethod::Head,
        NetworkSameSiteHttpMethod::Post => cookie_store::SameSiteContextHttpMethod::Post,
        NetworkSameSiteHttpMethod::Put => cookie_store::SameSiteContextHttpMethod::Put,
        NetworkSameSiteHttpMethod::Delete => cookie_store::SameSiteContextHttpMethod::Delete,
        NetworkSameSiteHttpMethod::Connect => cookie_store::SameSiteContextHttpMethod::Connect,
        NetworkSameSiteHttpMethod::Options => cookie_store::SameSiteContextHttpMethod::Options,
        NetworkSameSiteHttpMethod::Trace => cookie_store::SameSiteContextHttpMethod::Trace,
        NetworkSameSiteHttpMethod::Patch => cookie_store::SameSiteContextHttpMethod::Patch,
    };
    context.redirect_type = match request_context.site_context_metadata.context.redirect_type {
        NetworkSameSiteRedirectType::Unset => cookie_store::SameSiteContextRedirectType::Unset,
        NetworkSameSiteRedirectType::NoRedirect => {
            cookie_store::SameSiteContextRedirectType::NoRedirect
        }
        NetworkSameSiteRedirectType::CrossSiteRedirect => {
            cookie_store::SameSiteContextRedirectType::CrossSiteRedirect
        }
        NetworkSameSiteRedirectType::PartialSameSiteRedirect => {
            cookie_store::SameSiteContextRedirectType::PartialSameSiteRedirect
        }
        NetworkSameSiteRedirectType::AllSameSiteRedirect => {
            cookie_store::SameSiteContextRedirectType::AllSameSiteRedirect
        }
    };
    context.request_type = match request_context.request_type {
        NetworkCookieRequestType::Subresource => HttpRequestType::Subresource,
        NetworkCookieRequestType::TopLevelNavigation => HttpRequestType::TopLevelNavigation,
    };
    context.is_method_safe = request_context.is_method_safe;
    context
}

fn document_query_context<'a>(
    url: &'a Url,
    browser_context: &BrowserCookieFacadeContext,
) -> QueryContext<'a> {
    let mut context = QueryContext::document(url);
    context.browser_context = core_browser_site_context_from_facade(browser_context);
    context.browser_context.cookie_partition_key =
        Some(core_cookie_partition_key_for_url(browser_context, url));
    if let Some(site_basis_url) = browser_context.site_basis_url() {
        let schemeless_same_site = same_site_urls(url, site_basis_url, false);
        let schemeful_same_site = same_site_urls(url, site_basis_url, true);
        context.same_site_context = SameSiteContext::new(
            if schemeless_same_site {
                SameSiteRequestContext::SameSiteLax
            } else {
                SameSiteRequestContext::CrossSite
            },
            if schemeful_same_site {
                SameSiteRequestContext::SameSiteLax
            } else {
                SameSiteRequestContext::CrossSite
            },
        );
    }
    context
}

fn effective_document_cookie_url(url: &Url) -> Cow<'_, Url> {
    match url.scheme() {
        "blob" => Url::parse(url.path())
            .map(Cow::Owned)
            .unwrap_or_else(|_| Cow::Borrowed(url)),
        _ => Cow::Borrowed(url),
    }
}

fn document_insert_context<'a>(
    url: &'a Url,
    browser_context: &BrowserCookieFacadeContext,
) -> InsertContext<'a> {
    let mut context = InsertContext::document(url);
    context.browser_context = core_browser_site_context_from_facade(browser_context);
    context.browser_context.cookie_partition_key =
        Some(core_cookie_partition_key_for_url(browser_context, url));
    context
}

fn network_insert_context<'a>(
    url: &'a Url,
    request_context: &NetworkCookieRequestContext,
) -> InsertContext<'a> {
    let mut context = InsertContext::http(url);
    context.browser_context =
        core_browser_site_context_from_facade(&request_context.browser_context);
    context.browser_context.cookie_partition_key = Some(core_cookie_partition_key_for_url(
        &request_context.browser_context,
        url,
    ));
    context
}

fn insert_context<'a>(url: &'a Url, source: CookieSource) -> InsertContext<'a> {
    match source {
        #[cfg(any(test, feature = "test-support"))]
        CookieSource::Http => InsertContext::http(url),
        // Browser management is privileged. Treat it closer to Chromium's manager-side set path
        // than to `document.cookie`, so fork-side HttpOnly guards do not reject it.
        CookieSource::Management => InsertContext::cdp(url),
    }
}
