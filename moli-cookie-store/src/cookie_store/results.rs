use crate::cookie::Cookie;
use crate::CookieError;
use cookie::SameSite;
use time::{Duration, OffsetDateTime};

use super::{BrowserSiteContext, SameSiteContext, SameSiteContextMetadata};

/// The reason a cookie write was rejected.
///
/// This mirrors the browser-policy and parsing/storage rejection surface that
/// currently exists in `CookieError`, but gives higher layers a stable status
/// model that can evolve independently from the legacy `Result<StoreAction,
/// CookieError>` API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSetRejectionReason {
    /// A `HttpOnly` cookie was written through a non-HTTP API such as
    /// `document.cookie`.
    NonHttpScheme,
    /// A `Secure` cookie was written from a non-secure context.
    SecureOnly,
    /// The request URL had no relative-scheme host from which a cookie host
    /// could be derived.
    NonRelativeScheme,
    /// The request URL does not domain-match the cookie.
    DomainMismatch,
    /// `SameSite=None` cookies must also specify `Secure`.
    SameSiteNoneRequiresSecure,
    /// The cookie violated one of the protected prefix rules.
    PrefixViolation,
    /// An insecure write attempted to overlap an existing secure cookie.
    SecureOverlay,
    /// The cookie name and value exceed the supported browser-compatible size
    /// limit.
    NameValueTooLarge,
    /// A `Partitioned` cookie was written without a browser-computed key.
    PartitionedMissingPartitionKey,
    /// A `Partitioned` cookie did not also specify `Secure`.
    PartitionedRequiresSecure,
    /// Compatibility-only result retained for older callers.
    UnsupportedPartitioned,
    /// The cookie store was full and could not evict a cookie under its
    /// configured policy.
    StorageFull,
    /// The incoming cookie was immediately expired and did not expire an
    /// existing stored cookie.
    Expired,
    /// The input string could not be parsed as a cookie.
    Parse,
    #[cfg(feature = "public_suffix")]
    /// The cookie targeted a public suffix that must be rejected.
    PublicSuffix,
    /// The cookie had no canonical domain in a context that requires one.
    UnspecifiedDomain,
}

impl From<CookieError> for CookieSetRejectionReason {
    fn from(error: CookieError) -> Self {
        match error {
            CookieError::NonHttpScheme => Self::NonHttpScheme,
            CookieError::SecureOnly => Self::SecureOnly,
            CookieError::NonRelativeScheme => Self::NonRelativeScheme,
            CookieError::DomainMismatch => Self::DomainMismatch,
            CookieError::SameSiteNoneRequiresSecure => Self::SameSiteNoneRequiresSecure,
            CookieError::PrefixViolation => Self::PrefixViolation,
            CookieError::SecureOverlay => Self::SecureOverlay,
            CookieError::NameValueTooLarge => Self::NameValueTooLarge,
            CookieError::PartitionedMissingPartitionKey => Self::PartitionedMissingPartitionKey,
            CookieError::PartitionedRequiresSecure => Self::PartitionedRequiresSecure,
            CookieError::UnsupportedPartitioned => Self::UnsupportedPartitioned,
            CookieError::StorageFull => Self::StorageFull,
            CookieError::Expired => Self::Expired,
            CookieError::Parse => Self::Parse,
            #[cfg(feature = "public_suffix")]
            CookieError::PublicSuffix => Self::PublicSuffix,
            CookieError::UnspecifiedDomain => Self::UnspecifiedDomain,
        }
    }
}

/// Browser-style result for a cookie write.
///
/// This keeps accepted storage actions and rejected writes in one explicit
/// status enum so browser-facing callers do not need to reconstruct intent from
/// `Result` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSetResult {
    /// The cookie write was accepted by the store.
    Accepted(super::StoreAction),
    /// The cookie write was rejected.
    Rejected(CookieSetRejectionReason),
}

impl CookieSetResult {
    pub(super) fn into_insert_result(self) -> super::InsertResult {
        match self {
            Self::Accepted(action) => Ok(action),
            Self::Rejected(reason) => Err(reason.into_cookie_error()),
        }
    }
}

/// Non-fatal diagnostics attached to a cookie write.
///
/// Chromium exposes accepted-with-warning style results for several cookie
/// mutations. This fork starts with the sanitization cases that already exist
/// in practice today, instead of inventing a wide warning surface before the
/// core has concrete behavior to back it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSetWarningReason {
    /// The incoming `Domain` attribute was ignored because it exceeded the
    /// supported browser-compatible size or contained invalid octets.
    DomainAttributeIgnored,
    /// The incoming `Path` attribute was ignored because it exceeded the
    /// supported browser-compatible size or contained invalid octets.
    PathAttributeIgnored,
    /// A `Secure` cookie was accepted from a trustworthy but
    /// non-cryptographic URL such as `http://localhost`.
    ///
    /// Chromium surfaces this separately from hard `SecureOnly` rejection so
    /// callers can tell the difference between "rejected as insecure" and
    /// "allowed because the origin is treated as trustworthy".
    SecureAccessGrantedNonCryptographic,
}

/// Browser-style rich result for a cookie write.
///
/// This extends the legacy accepted/rejected status with non-fatal warnings
/// and the effective SameSite semantics of the resulting canonical cookie when
/// parsing succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieSetAccessResult {
    /// The accepted/rejected storage outcome.
    pub status: CookieSetResult,
    /// Ordered rejection reasons attached to the write.
    ///
    /// `status` remains the compatibility projection that exposes only the
    /// first rejection. New browser-facing callers should prefer this richer
    /// list when they need Chromium-style "why else would this write have been
    /// rejected?" diagnostics.
    pub rejection_reasons: Vec<CookieSetRejectionReason>,
    /// Non-fatal diagnostics gathered while handling the write.
    pub warning_reasons: Vec<CookieSetWarningReason>,
    /// The effective SameSite semantics of the parsed canonical cookie when
    /// parsing succeeded.
    pub effective_same_site: Option<CookieEffectiveSameSite>,
}

impl CookieSetAccessResult {
    /// Return true when the write was accepted by the store.
    pub fn is_accepted(&self) -> bool {
        matches!(self.status, CookieSetResult::Accepted(_))
    }

    /// Return true when the write completed with non-fatal warnings.
    pub fn has_warnings(&self) -> bool {
        !self.warning_reasons.is_empty()
    }

    pub(super) fn add_warning(&mut self, reason: CookieSetWarningReason) {
        if !self.warning_reasons.contains(&reason) {
            self.warning_reasons.push(reason);
        }
    }

    pub(super) fn add_rejection(&mut self, reason: CookieSetRejectionReason) {
        if !self.rejection_reasons.contains(&reason) {
            self.rejection_reasons.push(reason);
        }
        if !matches!(self.status, CookieSetResult::Rejected(_)) {
            self.status = CookieSetResult::Rejected(reason);
        }
    }

    pub(super) fn into_set_result(self) -> CookieSetResult {
        self.status
    }

    pub(super) fn into_insert_result(self) -> super::InsertResult {
        self.status.into_insert_result()
    }
}

impl CookieSetRejectionReason {
    pub(super) fn into_cookie_error(self) -> CookieError {
        match self {
            Self::NonHttpScheme => CookieError::NonHttpScheme,
            Self::SecureOnly => CookieError::SecureOnly,
            Self::NonRelativeScheme => CookieError::NonRelativeScheme,
            Self::DomainMismatch => CookieError::DomainMismatch,
            Self::SameSiteNoneRequiresSecure => CookieError::SameSiteNoneRequiresSecure,
            Self::PrefixViolation => CookieError::PrefixViolation,
            Self::SecureOverlay => CookieError::SecureOverlay,
            Self::NameValueTooLarge => CookieError::NameValueTooLarge,
            Self::PartitionedMissingPartitionKey => CookieError::PartitionedMissingPartitionKey,
            Self::PartitionedRequiresSecure => CookieError::PartitionedRequiresSecure,
            Self::UnsupportedPartitioned => CookieError::UnsupportedPartitioned,
            Self::StorageFull => CookieError::StorageFull,
            Self::Expired => CookieError::Expired,
            Self::Parse => CookieError::Parse,
            #[cfg(feature = "public_suffix")]
            Self::PublicSuffix => CookieError::PublicSuffix,
            Self::UnspecifiedDomain => CookieError::UnspecifiedDomain,
        }
    }
}

/// The reason a cookie was not included in a query result.
///
/// This is intentionally much smaller than Chromium's
/// `CookieInclusionStatus`/`CookieAccessResult` surface. The goal of this
/// first version is to make browser-style exclusion observable without
/// prematurely freezing a large status taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieExclusionReason {
    /// The cookie exists in the store but is expired.
    Expired,
    /// The cookie's canonical domain does not match the queried URL.
    DomainMismatch,
    /// The cookie's path does not match the queried URL.
    PathMismatch,
    /// The cookie is `Secure` but the queried URL is not considered secure.
    SecureOnly,
    /// The cookie is `HttpOnly` and the query context cannot observe it.
    HttpOnly,
    /// The cookie's recorded source port does not match the queried URL.
    PortMismatch,
    /// The cookie's recorded source scheme does not match the queried URL.
    SchemeMismatch,
    /// The cookie's `SameSite` setting excludes it from the current request
    /// context.
    SameSiteStrict,
    /// The cookie's explicit `SameSite=Lax` setting excludes it from the
    /// current request context.
    SameSiteLax,
    /// An omitted SameSite attribute was defaulted to Lax for this request.
    SameSiteUnspecifiedTreatedAsLax,
    /// A partitioned cookie belongs to a different top-level-site partition.
    PartitionKeyMismatch,
}

/// Non-fatal query diagnostics attached to a cookie access result.
///
/// Chromium distinguishes exclusions from warnings so callers can observe
/// policy-sensitive transitions without treating them as hard failures. This
/// fork keeps that shape intentionally small for now and grows it only where a
/// concrete browser-facing behavior already depends on the extra detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieWarningReason {
    /// Schemeless and schemeful SameSite evaluation disagreed for this cookie
    /// access.
    ///
    /// The current query path includes/excludes cookies using the stricter
    /// schemeful relation, but this warning preserves the fact that a legacy
    /// schemeless model would have made a different decision.
    SchemefulSameSiteContextMismatch,
    /// Schemeful evaluation downgraded a `SameSite=Strict` cookie from Strict
    /// to Lax.
    StrictLaxDowngradeStrictSameSite,
    /// Schemeful evaluation downgraded a `SameSite=Strict` cookie from Strict
    /// to Cross-Site.
    StrictCrossDowngradeStrictSameSite,
    /// Schemeful evaluation downgraded a `SameSite=Lax` cookie from Strict to
    /// Cross-Site.
    StrictCrossDowngradeLaxSameSite,
    /// Schemeful evaluation downgraded a `SameSite=Strict` cookie from Lax to
    /// Cross-Site.
    LaxCrossDowngradeStrictSameSite,
    /// Schemeful evaluation downgraded a `SameSite=Lax` cookie from Lax to
    /// Cross-Site.
    LaxCrossDowngradeLaxSameSite,
    /// A redirect chain downgraded the request from same-site to cross-site
    /// before this cookie access was evaluated.
    SameSiteContextDowngradedByRedirect,
    /// A `Secure` cookie was included for a trustworthy but
    /// non-cryptographic URL such as `http://localhost`.
    ///
    /// Keep this as a warning instead of an exclusion so callers can observe
    /// that the cookie was allowed only because the current origin is treated
    /// as trustworthy, not because it was genuinely cryptographic.
    SecureAccessGrantedNonCryptographic,
}

/// Effective SameSite semantics applied to a cookie during access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieEffectiveSameSite {
    /// The cookie is not restricted by SameSite.
    NoRestriction,
    /// The cookie behaves as `SameSite=Lax`.
    Lax,
    /// A recently-created default cookie also allows unsafe top-level navigation.
    LaxAllowUnsafe,
    /// The cookie behaves as `SameSite=Strict`.
    Strict,
}

impl CookieEffectiveSameSite {
    /// Applies the browser default without changing the stored SameSite attribute.
    /// The two-minute allowance applies only to cookies without an explicit mode.
    pub fn for_cookie(
        same_site: Option<SameSite>,
        creation_time: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> Self {
        let creation_time = creation_time.and_then(crate::cookie_creation_time::normalize);
        let now = crate::cookie_creation_time::normalize(now);
        match same_site {
            Some(SameSite::None) => Self::NoRestriction,
            Some(SameSite::Lax) => Self::Lax,
            Some(SameSite::Strict) => Self::Strict,
            None if creation_time
                .zip(now)
                .is_some_and(|(created, now)| now - created <= Duration::minutes(2)) =>
            {
                Self::LaxAllowUnsafe
            }
            None => Self::Lax,
        }
    }
}

/// Access semantics applied to a cookie access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieAccessSemantics {
    /// The current fork cannot yet determine a stable access semantics value.
    Unknown,
    /// The cookie is evaluated under the modern nonlegacy access model.
    NonLegacy,
    /// The cookie is evaluated under a legacy compatibility model.
    Legacy,
}

/// Scope semantics applied to a cookie access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieScopeSemantics {
    /// The current fork cannot yet determine a stable scope semantics value.
    Unknown,
    /// The cookie is evaluated under the modern nonlegacy scope model.
    NonLegacy,
    /// The cookie is evaluated under a legacy scope model.
    Legacy,
}

/// Inclusion/exclusion/warning status for one cookie access.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CookieInclusionStatus {
    /// Reasons this cookie was excluded from the result.
    pub exclusion_reasons: Vec<CookieExclusionReason>,
    /// Non-fatal diagnostics attached to the access.
    pub warning_reasons: Vec<CookieWarningReason>,
}

impl CookieInclusionStatus {
    /// Return true when no exclusion reason was recorded.
    pub fn is_included(&self) -> bool {
        self.exclusion_reasons.is_empty()
    }

    /// Return true when one or more warnings were recorded.
    pub fn has_warnings(&self) -> bool {
        !self.warning_reasons.is_empty()
    }

    pub(super) fn first_exclusion_reason(&self) -> Option<CookieExclusionReason> {
        self.exclusion_reasons.first().copied()
    }

    pub(super) fn add_exclusion(&mut self, reason: CookieExclusionReason) {
        if !self.exclusion_reasons.contains(&reason) {
            self.exclusion_reasons.push(reason);
        }
    }

    pub(super) fn add_warning(&mut self, reason: CookieWarningReason) {
        if !self.warning_reasons.contains(&reason) {
            self.warning_reasons.push(reason);
        }
    }
}

/// Browser-style access result for one cookie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieAccessResult {
    /// Inclusion/exclusion/warning status for this cookie access.
    pub status: CookieInclusionStatus,
    /// Effective SameSite semantics applied to this cookie.
    pub effective_same_site: CookieEffectiveSameSite,
    /// The request SameSite context evaluated for this access.
    pub same_site_context: SameSiteContext,
    /// SameSite context metadata attached to this access.
    pub same_site_context_metadata: SameSiteContextMetadata,
    /// Access semantics applied while evaluating this cookie.
    pub access_semantics: CookieAccessSemantics,
    /// Scope semantics applied while evaluating this cookie.
    pub scope_semantics: CookieScopeSemantics,
    /// Whether this query context is allowed to access secure cookies at all.
    pub is_allowed_to_access_secure_cookies: bool,
    /// Browser-side site context snapshot attached to this access.
    pub browser_context: BrowserSiteContext,
}

/// One cookie together with its access result.
#[derive(Debug, Clone, PartialEq)]
pub struct CookieWithAccessResult {
    /// The canonical cookie snapshot.
    pub cookie: Cookie<'static>,
    /// Access metadata describing how the query treated this cookie.
    pub access_result: CookieAccessResult,
}

/// Rich browser-style query result carrying per-cookie access metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CookieAccessQueryResult {
    /// Cookies included by the query, together with their access metadata.
    pub included_cookies: Vec<CookieWithAccessResult>,
    /// Cookies considered by the query but excluded, together with their
    /// access metadata.
    pub excluded_cookies: Vec<CookieWithAccessResult>,
}

/// A cookie excluded from a query result, together with the reason it was not
/// included.
#[derive(Debug, Clone, PartialEq)]
pub struct ExcludedCookie {
    /// The excluded canonical cookie snapshot.
    pub cookie: Cookie<'static>,
    /// The first exclusion reason projected through the compatibility query
    /// result model.
    pub reason: CookieExclusionReason,
}

/// Browser-style query result with both included and excluded cookies.
///
/// `excluded_cookies` is intentionally scoped to cookies whose domain matches
/// the queried URL. This keeps the result useful for diagnostics without
/// dumping the entire store for every request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CookieQueryResult {
    /// Cookies included by the query.
    pub included_cookies: Vec<Cookie<'static>>,
    /// Cookies considered by the query but excluded, projected into the
    /// compatibility result model.
    pub excluded_cookies: Vec<ExcludedCookie>,
}
