//! Request-side cookie access diagnostics.

use time::OffsetDateTime;
use url::Url;

use super::stored_cookie::{StoredCookie, StoredCookieSameSite};

/// SameSite value after browser policy has normalized unspecified attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieEffectiveSameSite {
    NoRestriction,
    Lax,
    LaxAllowUnsafe,
    Strict,
}

impl StoredCookieEffectiveSameSite {
    pub fn for_cookie(
        same_site: StoredCookieSameSite,
        creation_time: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> Self {
        let same_site = match same_site {
            StoredCookieSameSite::Unspecified => None,
            StoredCookieSameSite::None => Some(cookie_store::SameSite::None),
            StoredCookieSameSite::Lax => Some(cookie_store::SameSite::Lax),
            StoredCookieSameSite::Strict => Some(cookie_store::SameSite::Strict),
        };
        super::core_conversion::stored_effective_same_site_from_core(
            cookie_store::CookieEffectiveSameSite::for_cookie(same_site, creation_time, now),
        )
    }
}

/// Whether the cookie is evaluated under legacy or modern access semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieAccessSemantics {
    Unknown,
    NonLegacy,
    Legacy,
}

/// Whether the cookie's scope should be interpreted with legacy compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieScopeSemantics {
    Unknown,
    NonLegacy,
    Legacy,
}

/// Storage Access API state used while deciding whether a cookie is readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieStorageAccessStatus {
    None,
    Granted,
}

/// Browser context value that supplied the site comparison basis for a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSiteContextBasis {
    None,
    SiteForCookies,
    TopFrameOrigin,
}

/// Where a browser-context value came from when the query report was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieBrowserContextValueSource {
    Unset,
    RequestContext,
    FacadeDefault,
    FacadeOverride,
}

/// Non-fatal diagnostics for cookies considered during a request lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieWarningReason {
    SchemefulSameSiteContextMismatch,
    StrictLaxDowngradeStrictSameSite,
    StrictCrossDowngradeStrictSameSite,
    StrictCrossDowngradeLaxSameSite,
    LaxCrossDowngradeStrictSameSite,
    LaxCrossDowngradeLaxSameSite,
    SameSiteContextDowngradedByRedirect,
    SecureAccessGrantedNonCryptographic,
}

/// SameSite request context used for one cookie-access decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieRequestSameSiteContext {
    SameSiteStrict,
    SameSiteLax,
    SameSiteLaxMethodUnsafe,
    CrossSite,
}

/// How a redirect chain weakened the SameSite context for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSameSiteContextDowngradeType {
    StrictToLax,
    StrictToCross,
    LaxToCross,
}

/// HTTP method attached to SameSite diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSameSiteHttpMethod {
    Unset,
    Unknown,
    Get,
    Head,
    Post,
    Put,
    Delete,
    Connect,
    Options,
    Trace,
    Patch,
}

/// Redirect-chain classification attached to SameSite diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieSameSiteRedirectType {
    Unset,
    NoRedirect,
    CrossSiteRedirect,
    PartialSameSiteRedirect,
    AllSameSiteRedirect,
}

/// Reasons a cookie was not visible to a request or document-cookie query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredCookieExclusionReason {
    CookiesDisabled,
    StorageAccessBlocked,
    StoreUnavailable,
    Expired,
    DomainMismatch,
    PathMismatch,
    SecureOnly,
    HttpOnly,
    PortMismatch,
    SchemeMismatch,
    SameSiteStrict,
    SameSiteLax,
    SameSiteUnspecifiedTreatedAsLax,
    PartitionKeyMismatch,
}

/// Facade-level availability state before individual cookie matching runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCookieFacadeStatus {
    /// Whether the caller's browser facade allowed cookie access at all.
    pub cookie_access_enabled: bool,
    /// Whether the underlying canonical store was available for this query.
    pub store_available: bool,
    /// Global blockers that apply even before per-cookie matching decisions.
    pub blocked_reasons: Vec<StoredCookieExclusionReason>,
}

impl Default for StoredCookieFacadeStatus {
    fn default() -> Self {
        Self {
            cookie_access_enabled: true,
            store_available: true,
            blocked_reasons: Vec::new(),
        }
    }
}

/// Per-cookie access diagnostics for a single request-side lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCookieAccess {
    /// Cookie that was considered by the canonical store.
    pub cookie: StoredCookie,
    /// Blocking reasons for this cookie; empty means the cookie was included.
    pub exclusion_reasons: Vec<StoredCookieExclusionReason>,
    /// Non-fatal compatibility or policy warnings for this cookie.
    pub warning_reasons: Vec<StoredCookieWarningReason>,
    /// Effective SameSite value after defaulting and policy normalization.
    pub effective_same_site: StoredCookieEffectiveSameSite,
    /// Schemeless SameSite context used by legacy-compatible logic.
    pub same_site_context: StoredCookieRequestSameSiteContext,
    /// Schemeful SameSite context used by modern browser logic.
    pub schemeful_same_site_context: StoredCookieRequestSameSiteContext,
    /// Redirect downgrade for the schemeless track, if one occurred.
    pub same_site_context_downgrade_type: Option<StoredCookieSameSiteContextDowngradeType>,
    /// Redirect downgrade for the schemeful track, if one occurred.
    pub schemeful_same_site_context_downgrade_type:
        Option<StoredCookieSameSiteContextDowngradeType>,
    /// HTTP method recorded for the schemeless SameSite track.
    pub same_site_context_http_method: StoredCookieSameSiteHttpMethod,
    /// HTTP method recorded for the schemeful SameSite track.
    pub schemeful_same_site_context_http_method: StoredCookieSameSiteHttpMethod,
    /// Redirect classification recorded for the schemeless SameSite track.
    pub same_site_context_redirect_type: StoredCookieSameSiteRedirectType,
    /// Redirect classification recorded for the schemeful SameSite track.
    pub schemeful_same_site_context_redirect_type: StoredCookieSameSiteRedirectType,
    /// Access semantics emitted by the canonical cookie engine.
    pub access_semantics: StoredCookieAccessSemantics,
    /// Scope semantics emitted by the canonical cookie engine.
    pub scope_semantics: StoredCookieScopeSemantics,
    /// Whether this query was allowed to read Secure cookies on the URL.
    pub is_allowed_to_access_secure_cookies: bool,
    /// Site-for-cookies URL used by the browser facade, if available.
    pub site_for_cookies_url: Option<Url>,
    /// Source of `site_for_cookies_url`.
    pub site_for_cookies_source: StoredCookieBrowserContextValueSource,
    /// Top-frame origin URL used by the browser facade, if available.
    pub top_frame_origin_url: Option<Url>,
    /// Source of `top_frame_origin_url`.
    pub top_frame_origin_source: StoredCookieBrowserContextValueSource,
    /// Storage access state used for this decision.
    pub storage_access_status: StoredCookieStorageAccessStatus,
    /// Source of `storage_access_status`.
    pub storage_access_status_source: StoredCookieBrowserContextValueSource,
    /// Which browser-context value ultimately drove SameSite comparison.
    pub site_context_basis: StoredCookieSiteContextBasis,
}

/// Records the cookie lookup outcome for one outgoing request: cookies that
/// were attached, cookies that were considered but blocked, and any facade-level
/// reason cookie access was unavailable. This is request-side observability, not
/// response Set-Cookie processing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredCookieQueryReport {
    /// Store/facade availability status for the whole lookup.
    pub facade_status: StoredCookieFacadeStatus,
    /// Lookup-wide blockers projected for callers that do not inspect status.
    pub facade_exclusion_reasons: Vec<StoredCookieExclusionReason>,
    /// Cookies that were attached to the request or exposed to document.cookie.
    pub included_cookies: Vec<StoredCookieAccess>,
    /// Cookies considered by the store but blocked by policy or matching rules.
    pub excluded_cookies: Vec<StoredCookieAccess>,
}
