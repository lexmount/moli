use cookie_store::{
    BrowserSiteContext as CoreBrowserSiteContext, Cookie as StoreCookie,
    CookieAccessQueryResult as CoreCookieAccessQueryResult,
    CookieAccessSemantics as CoreCookieAccessSemantics,
    CookieEffectiveSameSite as CoreCookieEffectiveSameSite,
    CookieExclusionReason as CoreCookieExclusionReason,
    CookieScopeSemantics as CoreCookieScopeSemantics,
    CookieSetAccessResult as CoreCookieSetAccessResult,
    CookieSetRejectionReason as CoreCookieSetRejectionReason,
    CookieSetResult as CoreCookieSetResult, CookieSetWarningReason as CoreCookieSetWarningReason,
    CookieWarningReason as CoreCookieWarningReason,
    CookieWithAccessResult as CoreCookieWithAccessResult,
    SameSiteContextDowngradeType as CoreSameSiteContextDowngradeType,
    SameSiteContextHttpMethod as CoreSameSiteContextHttpMethod,
    SameSiteContextRedirectType as CoreSameSiteContextRedirectType,
    SameSiteRequestContext as CoreSameSiteRequestContext,
    StorageAccessStatus as CoreStorageAccessStatus,
};

use super::query_report::{
    StoredCookieAccess, StoredCookieAccessSemantics, StoredCookieBrowserContextValueSource,
    StoredCookieEffectiveSameSite, StoredCookieExclusionReason, StoredCookieFacadeStatus,
    StoredCookieQueryReport, StoredCookieRequestSameSiteContext,
    StoredCookieSameSiteContextDowngradeType, StoredCookieSameSiteHttpMethod,
    StoredCookieSameSiteRedirectType, StoredCookieScopeSemantics, StoredCookieSiteContextBasis,
    StoredCookieStorageAccessStatus, StoredCookieWarningReason,
};
use super::set_report::{
    StoredCookieSetRejectionReason, StoredCookieSetReport, StoredCookieSetStatus,
    StoredCookieSetWarningReason,
};
use super::stored_cookie::{StoredCookie, cookie_expiration, stored_source_scheme_from_core};

fn stored_request_same_site_context_from_core(
    context: CoreSameSiteRequestContext,
) -> StoredCookieRequestSameSiteContext {
    match context {
        CoreSameSiteRequestContext::SameSiteStrict => {
            StoredCookieRequestSameSiteContext::SameSiteStrict
        }
        CoreSameSiteRequestContext::SameSiteLax => StoredCookieRequestSameSiteContext::SameSiteLax,
        CoreSameSiteRequestContext::SameSiteLaxMethodUnsafe => {
            StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe
        }
        CoreSameSiteRequestContext::CrossSite => StoredCookieRequestSameSiteContext::CrossSite,
    }
}

pub(super) fn stored_effective_same_site_from_core(
    same_site: CoreCookieEffectiveSameSite,
) -> StoredCookieEffectiveSameSite {
    match same_site {
        CoreCookieEffectiveSameSite::NoRestriction => StoredCookieEffectiveSameSite::NoRestriction,
        CoreCookieEffectiveSameSite::Lax => StoredCookieEffectiveSameSite::Lax,
        CoreCookieEffectiveSameSite::LaxAllowUnsafe => {
            StoredCookieEffectiveSameSite::LaxAllowUnsafe
        }
        CoreCookieEffectiveSameSite::Strict => StoredCookieEffectiveSameSite::Strict,
    }
}

pub(super) fn stored_access_semantics_from_core(
    semantics: CoreCookieAccessSemantics,
) -> StoredCookieAccessSemantics {
    match semantics {
        CoreCookieAccessSemantics::Unknown => StoredCookieAccessSemantics::Unknown,
        CoreCookieAccessSemantics::NonLegacy => StoredCookieAccessSemantics::NonLegacy,
        CoreCookieAccessSemantics::Legacy => StoredCookieAccessSemantics::Legacy,
    }
}

pub(super) fn stored_scope_semantics_from_core(
    semantics: CoreCookieScopeSemantics,
) -> StoredCookieScopeSemantics {
    match semantics {
        CoreCookieScopeSemantics::Unknown => StoredCookieScopeSemantics::Unknown,
        CoreCookieScopeSemantics::NonLegacy => StoredCookieScopeSemantics::NonLegacy,
        CoreCookieScopeSemantics::Legacy => StoredCookieScopeSemantics::Legacy,
    }
}

pub(super) fn stored_storage_access_status_from_core(
    status: CoreStorageAccessStatus,
) -> StoredCookieStorageAccessStatus {
    match status {
        CoreStorageAccessStatus::None => StoredCookieStorageAccessStatus::None,
        CoreStorageAccessStatus::Granted => StoredCookieStorageAccessStatus::Granted,
    }
}

fn stored_site_context_basis_from_core(
    context: &CoreBrowserSiteContext,
) -> StoredCookieSiteContextBasis {
    if context.site_for_cookies_url.is_some() {
        StoredCookieSiteContextBasis::SiteForCookies
    } else if context.top_frame_origin_url.is_some() {
        StoredCookieSiteContextBasis::TopFrameOrigin
    } else {
        StoredCookieSiteContextBasis::None
    }
}

fn stored_browser_context_value_source_from_core_presence(
    present: bool,
) -> StoredCookieBrowserContextValueSource {
    if present {
        StoredCookieBrowserContextValueSource::RequestContext
    } else {
        StoredCookieBrowserContextValueSource::Unset
    }
}

pub(super) fn stored_warning_reason_from_core(
    reason: CoreCookieWarningReason,
) -> StoredCookieWarningReason {
    match reason {
        CoreCookieWarningReason::SchemefulSameSiteContextMismatch => {
            StoredCookieWarningReason::SchemefulSameSiteContextMismatch
        }
        CoreCookieWarningReason::StrictLaxDowngradeStrictSameSite => {
            StoredCookieWarningReason::StrictLaxDowngradeStrictSameSite
        }
        CoreCookieWarningReason::StrictCrossDowngradeStrictSameSite => {
            StoredCookieWarningReason::StrictCrossDowngradeStrictSameSite
        }
        CoreCookieWarningReason::StrictCrossDowngradeLaxSameSite => {
            StoredCookieWarningReason::StrictCrossDowngradeLaxSameSite
        }
        CoreCookieWarningReason::LaxCrossDowngradeStrictSameSite => {
            StoredCookieWarningReason::LaxCrossDowngradeStrictSameSite
        }
        CoreCookieWarningReason::LaxCrossDowngradeLaxSameSite => {
            StoredCookieWarningReason::LaxCrossDowngradeLaxSameSite
        }
        CoreCookieWarningReason::SameSiteContextDowngradedByRedirect => {
            StoredCookieWarningReason::SameSiteContextDowngradedByRedirect
        }
        CoreCookieWarningReason::SecureAccessGrantedNonCryptographic => {
            StoredCookieWarningReason::SecureAccessGrantedNonCryptographic
        }
    }
}

pub(super) fn stored_same_site_context_downgrade_type_from_core(
    downgrade_type: CoreSameSiteContextDowngradeType,
) -> StoredCookieSameSiteContextDowngradeType {
    match downgrade_type {
        CoreSameSiteContextDowngradeType::StrictToLax => {
            StoredCookieSameSiteContextDowngradeType::StrictToLax
        }
        CoreSameSiteContextDowngradeType::StrictToCross => {
            StoredCookieSameSiteContextDowngradeType::StrictToCross
        }
        CoreSameSiteContextDowngradeType::LaxToCross => {
            StoredCookieSameSiteContextDowngradeType::LaxToCross
        }
    }
}

pub(super) fn stored_same_site_http_method_from_core(
    http_method: CoreSameSiteContextHttpMethod,
) -> StoredCookieSameSiteHttpMethod {
    match http_method {
        CoreSameSiteContextHttpMethod::Unset => StoredCookieSameSiteHttpMethod::Unset,
        CoreSameSiteContextHttpMethod::Unknown => StoredCookieSameSiteHttpMethod::Unknown,
        CoreSameSiteContextHttpMethod::Get => StoredCookieSameSiteHttpMethod::Get,
        CoreSameSiteContextHttpMethod::Head => StoredCookieSameSiteHttpMethod::Head,
        CoreSameSiteContextHttpMethod::Post => StoredCookieSameSiteHttpMethod::Post,
        CoreSameSiteContextHttpMethod::Put => StoredCookieSameSiteHttpMethod::Put,
        CoreSameSiteContextHttpMethod::Delete => StoredCookieSameSiteHttpMethod::Delete,
        CoreSameSiteContextHttpMethod::Connect => StoredCookieSameSiteHttpMethod::Connect,
        CoreSameSiteContextHttpMethod::Options => StoredCookieSameSiteHttpMethod::Options,
        CoreSameSiteContextHttpMethod::Trace => StoredCookieSameSiteHttpMethod::Trace,
        CoreSameSiteContextHttpMethod::Patch => StoredCookieSameSiteHttpMethod::Patch,
    }
}

pub(super) fn stored_same_site_redirect_type_from_core(
    redirect_type: CoreSameSiteContextRedirectType,
) -> StoredCookieSameSiteRedirectType {
    match redirect_type {
        CoreSameSiteContextRedirectType::Unset => StoredCookieSameSiteRedirectType::Unset,
        CoreSameSiteContextRedirectType::NoRedirect => StoredCookieSameSiteRedirectType::NoRedirect,
        CoreSameSiteContextRedirectType::CrossSiteRedirect => {
            StoredCookieSameSiteRedirectType::CrossSiteRedirect
        }
        CoreSameSiteContextRedirectType::PartialSameSiteRedirect => {
            StoredCookieSameSiteRedirectType::PartialSameSiteRedirect
        }
        CoreSameSiteContextRedirectType::AllSameSiteRedirect => {
            StoredCookieSameSiteRedirectType::AllSameSiteRedirect
        }
    }
}

pub(super) fn stored_exclusion_reason_from_core(
    reason: CoreCookieExclusionReason,
) -> StoredCookieExclusionReason {
    match reason {
        CoreCookieExclusionReason::Expired => StoredCookieExclusionReason::Expired,
        CoreCookieExclusionReason::DomainMismatch => StoredCookieExclusionReason::DomainMismatch,
        CoreCookieExclusionReason::PathMismatch => StoredCookieExclusionReason::PathMismatch,
        CoreCookieExclusionReason::SecureOnly => StoredCookieExclusionReason::SecureOnly,
        CoreCookieExclusionReason::HttpOnly => StoredCookieExclusionReason::HttpOnly,
        CoreCookieExclusionReason::PortMismatch => StoredCookieExclusionReason::PortMismatch,
        CoreCookieExclusionReason::SchemeMismatch => StoredCookieExclusionReason::SchemeMismatch,
        CoreCookieExclusionReason::SameSiteStrict => StoredCookieExclusionReason::SameSiteStrict,
        CoreCookieExclusionReason::SameSiteLax => StoredCookieExclusionReason::SameSiteLax,
        CoreCookieExclusionReason::SameSiteUnspecifiedTreatedAsLax => {
            StoredCookieExclusionReason::SameSiteUnspecifiedTreatedAsLax
        }
        CoreCookieExclusionReason::PartitionKeyMismatch => {
            StoredCookieExclusionReason::PartitionKeyMismatch
        }
    }
}

pub(super) fn stored_set_warning_reason_from_core(
    reason: CoreCookieSetWarningReason,
) -> StoredCookieSetWarningReason {
    match reason {
        CoreCookieSetWarningReason::DomainAttributeIgnored => {
            StoredCookieSetWarningReason::DomainAttributeIgnored
        }
        CoreCookieSetWarningReason::PathAttributeIgnored => {
            StoredCookieSetWarningReason::PathAttributeIgnored
        }
        CoreCookieSetWarningReason::SecureAccessGrantedNonCryptographic => {
            StoredCookieSetWarningReason::SecureAccessGrantedNonCryptographic
        }
    }
}

pub(super) fn stored_set_status_from_core(status: CoreCookieSetResult) -> StoredCookieSetStatus {
    match status {
        CoreCookieSetResult::Accepted(action) => StoredCookieSetStatus::Accepted(action),
        CoreCookieSetResult::Rejected(reason) => {
            StoredCookieSetStatus::Rejected(stored_set_rejection_reason_from_core(reason))
        }
    }
}

fn stored_set_rejection_reason_from_core(
    reason: CoreCookieSetRejectionReason,
) -> StoredCookieSetRejectionReason {
    match reason {
        CoreCookieSetRejectionReason::NonHttpScheme => {
            StoredCookieSetRejectionReason::NonHttpScheme
        }
        CoreCookieSetRejectionReason::SecureOnly => StoredCookieSetRejectionReason::SecureOnly,
        CoreCookieSetRejectionReason::NonRelativeScheme => {
            StoredCookieSetRejectionReason::NonRelativeScheme
        }
        CoreCookieSetRejectionReason::DomainMismatch => {
            StoredCookieSetRejectionReason::DomainMismatch
        }
        CoreCookieSetRejectionReason::SameSiteNoneRequiresSecure => {
            StoredCookieSetRejectionReason::SameSiteNoneRequiresSecure
        }
        CoreCookieSetRejectionReason::SameSiteStrict => {
            StoredCookieSetRejectionReason::SameSiteStrict
        }
        CoreCookieSetRejectionReason::SameSiteLax => StoredCookieSetRejectionReason::SameSiteLax,
        CoreCookieSetRejectionReason::SameSiteUnspecifiedTreatedAsLax => {
            StoredCookieSetRejectionReason::SameSiteUnspecifiedTreatedAsLax
        }
        CoreCookieSetRejectionReason::PrefixViolation => {
            StoredCookieSetRejectionReason::PrefixViolation
        }
        CoreCookieSetRejectionReason::SecureOverlay => {
            StoredCookieSetRejectionReason::SecureOverlay
        }
        CoreCookieSetRejectionReason::NameValueTooLarge => {
            StoredCookieSetRejectionReason::NameValueTooLarge
        }
        CoreCookieSetRejectionReason::PartitionedMissingPartitionKey => {
            StoredCookieSetRejectionReason::PartitionedMissingPartitionKey
        }
        CoreCookieSetRejectionReason::PartitionedRequiresSecure => {
            StoredCookieSetRejectionReason::PartitionedRequiresSecure
        }
        CoreCookieSetRejectionReason::UnsupportedPartitioned => {
            StoredCookieSetRejectionReason::UnsupportedPartitioned
        }
        CoreCookieSetRejectionReason::StorageFull => StoredCookieSetRejectionReason::StorageFull,
        CoreCookieSetRejectionReason::Expired => StoredCookieSetRejectionReason::Expired,
        CoreCookieSetRejectionReason::Parse => StoredCookieSetRejectionReason::Parse,
        CoreCookieSetRejectionReason::PublicSuffix => StoredCookieSetRejectionReason::PublicSuffix,
        CoreCookieSetRejectionReason::UnspecifiedDomain => {
            StoredCookieSetRejectionReason::UnspecifiedDomain
        }
    }
}

pub fn stored_query_report_from_core(
    result: CoreCookieAccessQueryResult,
) -> StoredCookieQueryReport {
    StoredCookieQueryReport {
        facade_status: StoredCookieFacadeStatus::default(),
        facade_exclusion_reasons: Vec::new(),
        included_cookies: result
            .included_cookies
            .into_iter()
            .map(stored_cookie_access_from_core)
            .collect(),
        excluded_cookies: result
            .excluded_cookies
            .into_iter()
            .map(stored_cookie_access_from_core)
            .collect(),
    }
}

pub fn stored_set_report_from_core(result: CoreCookieSetAccessResult) -> StoredCookieSetReport {
    StoredCookieSetReport {
        status: stored_set_status_from_core(result.status),
        rejection_reasons: result
            .rejection_reasons
            .into_iter()
            .map(stored_set_rejection_reason_from_core)
            .collect(),
        warning_reasons: result
            .warning_reasons
            .into_iter()
            .map(stored_set_warning_reason_from_core)
            .collect(),
        effective_same_site: result
            .effective_same_site
            .map(stored_effective_same_site_from_core),
    }
}

fn stored_cookie_access_from_core(result: CoreCookieWithAccessResult) -> StoredCookieAccess {
    StoredCookieAccess {
        cookie: stored_cookie_from_core(&result.cookie),
        exclusion_reasons: result
            .access_result
            .status
            .exclusion_reasons
            .into_iter()
            .map(stored_exclusion_reason_from_core)
            .collect(),
        warning_reasons: result
            .access_result
            .status
            .warning_reasons
            .into_iter()
            .map(stored_warning_reason_from_core)
            .collect(),
        effective_same_site: stored_effective_same_site_from_core(
            result.access_result.effective_same_site,
        ),
        same_site_context: stored_request_same_site_context_from_core(
            result.access_result.same_site_context.context,
        ),
        schemeful_same_site_context: stored_request_same_site_context_from_core(
            result.access_result.same_site_context.schemeful_context,
        ),
        same_site_context_downgrade_type: result
            .access_result
            .same_site_context_metadata
            .context
            .downgrade_type
            .map(stored_same_site_context_downgrade_type_from_core),
        schemeful_same_site_context_downgrade_type: result
            .access_result
            .same_site_context_metadata
            .schemeful_context
            .downgrade_type
            .map(stored_same_site_context_downgrade_type_from_core),
        same_site_context_http_method: stored_same_site_http_method_from_core(
            result
                .access_result
                .same_site_context_metadata
                .context
                .http_method,
        ),
        schemeful_same_site_context_http_method: stored_same_site_http_method_from_core(
            result
                .access_result
                .same_site_context_metadata
                .schemeful_context
                .http_method,
        ),
        same_site_context_redirect_type: stored_same_site_redirect_type_from_core(
            result
                .access_result
                .same_site_context_metadata
                .context
                .redirect_type,
        ),
        schemeful_same_site_context_redirect_type: stored_same_site_redirect_type_from_core(
            result
                .access_result
                .same_site_context_metadata
                .schemeful_context
                .redirect_type,
        ),
        access_semantics: stored_access_semantics_from_core(result.access_result.access_semantics),
        scope_semantics: stored_scope_semantics_from_core(result.access_result.scope_semantics),
        is_allowed_to_access_secure_cookies: result
            .access_result
            .is_allowed_to_access_secure_cookies,
        site_for_cookies_url: result
            .access_result
            .browser_context
            .site_for_cookies_url
            .clone(),
        site_for_cookies_source: stored_browser_context_value_source_from_core_presence(
            result
                .access_result
                .browser_context
                .site_for_cookies_url
                .is_some(),
        ),
        top_frame_origin_url: result
            .access_result
            .browser_context
            .top_frame_origin_url
            .clone(),
        top_frame_origin_source: stored_browser_context_value_source_from_core_presence(
            result
                .access_result
                .browser_context
                .top_frame_origin_url
                .is_some(),
        ),
        storage_access_status: stored_storage_access_status_from_core(
            result.access_result.browser_context.storage_access_status,
        ),
        storage_access_status_source: StoredCookieBrowserContextValueSource::RequestContext,
        site_context_basis: stored_site_context_basis_from_core(
            &result.access_result.browser_context,
        ),
    }
}

pub fn stored_cookie_from_core(cookie: &StoreCookie<'_>) -> StoredCookie {
    StoredCookie {
        name: cookie.name().to_owned(),
        value: cookie.value().to_owned(),
        domain: match &cookie.domain {
            cookie_store::CookieDomain::HostOnly(domain)
            | cookie_store::CookieDomain::Suffix(domain) => domain.clone(),
            cookie_store::CookieDomain::NotPresent | cookie_store::CookieDomain::Empty => {
                String::new()
            }
        },
        host_only: matches!(cookie.domain, cookie_store::CookieDomain::HostOnly(_)),
        path: String::from(&cookie.path),
        secure: cookie.secure().unwrap_or(false),
        http_only: cookie.http_only().unwrap_or(false),
        expires: cookie_expiration(cookie),
        same_site: cookie
            .same_site()
            .map(super::stored_cookie::StoredCookieSameSite::from)
            .unwrap_or(super::stored_cookie::StoredCookieSameSite::Unspecified),
        priority: cookie.priority(),
        partition_key: cookie.partition_key().map(stored_partition_key_from_core),
        source_scheme: stored_source_scheme_from_core(cookie.source_scheme()),
        source_port: cookie.source_port(),
        creation_index: cookie.creation_index(),
        creation_time: cookie.creation_time(),
        last_access_index: cookie.last_access_index(),
    }
}

pub(crate) fn stored_partition_key_from_core(
    key: &cookie_store::CookiePartitionKey,
) -> super::StoredCookiePartitionKey {
    match key {
        cookie_store::CookiePartitionKey::Site {
            top_level_site,
            has_cross_site_ancestor,
        } => {
            super::StoredCookiePartitionKey::site(top_level_site.clone(), *has_cross_site_ancestor)
        }
        cookie_store::CookiePartitionKey::Opaque {
            nonce,
            has_cross_site_ancestor,
        } => super::StoredCookiePartitionKey::opaque(*nonce, *has_cross_site_ancestor),
    }
}

pub(crate) fn core_partition_key_from_stored(
    key: &super::StoredCookiePartitionKey,
) -> cookie_store::CookiePartitionKey {
    match key {
        super::StoredCookiePartitionKey::Site {
            top_level_site,
            has_cross_site_ancestor,
        } => {
            cookie_store::CookiePartitionKey::site(top_level_site.clone(), *has_cross_site_ancestor)
        }
        super::StoredCookiePartitionKey::Opaque {
            nonce,
            has_cross_site_ancestor,
        } => cookie_store::CookiePartitionKey::opaque(*nonce, *has_cross_site_ancestor),
    }
}
