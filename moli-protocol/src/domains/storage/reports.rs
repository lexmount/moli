use moli_cookie_jar::{
    StoredCookie, StoredCookieAccess, StoredCookieAccessSemantics,
    StoredCookieBrowserContextValueSource, StoredCookieEffectiveSameSite,
    StoredCookieExclusionReason, StoredCookieQueryReport, StoredCookieRequestSameSiteContext,
    StoredCookieSameSite, StoredCookieSameSiteContextDowngradeType, StoredCookieSameSiteHttpMethod,
    StoredCookieSameSiteRedirectType, StoredCookieScopeSemantics, StoredCookieSetRejectionReason,
    StoredCookieSetReport, StoredCookieSetStatus, StoredCookieSetWarningReason,
    StoredCookieSiteContextBasis, StoredCookieStorageAccessStatus, StoredCookieWarningReason,
};
use serde_json::{Value, json};
use url::Url;

pub(crate) fn storage_cookie_to_json(cookie: &StoredCookie) -> Value {
    let domain = if cookie.host_only {
        cookie.domain.clone()
    } else {
        format!(".{}", cookie.domain)
    };
    let mut value = json!({
        "name": cookie.name,
        "value": cookie.value,
        "domain": domain,
        "path": cookie.path,
        "expires": cookie
            .expires
            .map(cdp_timestamp_from_offset_datetime)
            .unwrap_or(-1.0),
        "size": cookie.name.len() + cookie.value.len(),
        "httpOnly": cookie.http_only,
        "secure": cookie.secure,
        "session": cookie.expires.is_none(),
        "sourceScheme": cookie.source_scheme.as_str(),
        "sourcePort": cookie.source_port,
        // CDP `Network.Cookie.priority` is required. A cookie set without a
        // `Priority` attribute reports Chromium's default, `Medium`.
        "priority": cookie.effective_priority().as_str(),
    });
    match cookie.same_site {
        StoredCookieSameSite::Unspecified => {}
        StoredCookieSameSite::None => value["sameSite"] = json!("None"),
        StoredCookieSameSite::Lax => value["sameSite"] = json!("Lax"),
        StoredCookieSameSite::Strict => value["sameSite"] = json!("Strict"),
    }
    if let Some(partition_key) = cookie.partition_key.as_ref() {
        match partition_key {
            moli_cookie_jar::StoredCookiePartitionKey::Site {
                top_level_site,
                has_cross_site_ancestor,
            } => {
                value["partitionKey"] = json!({
                    "topLevelSite": top_level_site,
                    "hasCrossSiteAncestor": has_cross_site_ancestor,
                });
            }
            moli_cookie_jar::StoredCookiePartitionKey::Opaque { .. } => {
                value["partitionKeyOpaque"] = json!(true);
            }
        }
    }
    value
}

pub(crate) fn storage_cookie_matches_url(cookie: &StoredCookie, url: &Url) -> bool {
    cookie.matches(url)
}

pub(crate) fn cookie_set_report_to_json(report: &StoredCookieSetReport) -> Value {
    json!({
        "status": cookie_set_status_to_json(&report.status),
        "rejectionReasons": report
            .rejection_reasons
            .iter()
            .map(|reason| cookie_set_rejection_reason_to_str(*reason))
            .collect::<Vec<_>>(),
        "warningReasons": report
            .warning_reasons
            .iter()
            .map(cookie_set_warning_reason_to_str)
            .collect::<Vec<_>>(),
        "effectiveSameSite": report.effective_same_site.map(cookie_effective_same_site_to_str),
    })
}

pub(crate) fn cookie_query_report_to_json(report: &StoredCookieQueryReport) -> Value {
    json!({
        "facadeStatus": {
            "cookieAccessEnabled": report.facade_status.cookie_access_enabled,
            "storeAvailable": report.facade_status.store_available,
            "blockedReasons": report
                .facade_status
                .blocked_reasons
                .iter()
                .map(cookie_exclusion_reason_to_str)
                .collect::<Vec<_>>(),
        },
        "facadeExclusionReasons": report
            .facade_exclusion_reasons
            .iter()
            .map(cookie_exclusion_reason_to_str)
            .collect::<Vec<_>>(),
        "includedCookies": report
            .included_cookies
            .iter()
            .map(cookie_access_to_json)
            .collect::<Vec<_>>(),
        "excludedCookies": report
            .excluded_cookies
            .iter()
            .map(cookie_access_to_json)
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn associated_cookies_to_json(report: &StoredCookieQueryReport) -> Vec<Value> {
    report
        .included_cookies
        .iter()
        .map(|access| {
            json!({
                "cookie": storage_cookie_to_json(&access.cookie),
                "blockedReasons": [],
            })
        })
        .chain(report.excluded_cookies.iter().map(|access| {
            json!({
                "cookie": storage_cookie_to_json(&access.cookie),
                "blockedReasons": access
                    .exclusion_reasons
                    .iter()
                    .map(cdp_cookie_blocked_reason_to_str)
                    .collect::<Vec<_>>(),
            })
        }))
        .collect()
}

fn cookie_access_to_json(access: &StoredCookieAccess) -> Value {
    json!({
        "cookie": storage_cookie_to_json(&access.cookie),
        "exclusionReasons": access
            .exclusion_reasons
            .iter()
            .map(cookie_exclusion_reason_to_str)
            .collect::<Vec<_>>(),
        "warningReasons": access
            .warning_reasons
            .iter()
            .map(cookie_warning_reason_to_str)
            .collect::<Vec<_>>(),
        "effectiveSameSite": cookie_effective_same_site_to_str(access.effective_same_site),
        "sameSiteContext": cookie_request_same_site_context_to_str(access.same_site_context),
        "schemefulSameSiteContext": cookie_request_same_site_context_to_str(
            access.schemeful_same_site_context,
        ),
        "sameSiteContextDowngradeType": access
            .same_site_context_downgrade_type
            .map(cookie_same_site_context_downgrade_type_to_str),
        "schemefulSameSiteContextDowngradeType": access
            .schemeful_same_site_context_downgrade_type
            .map(cookie_same_site_context_downgrade_type_to_str),
        "sameSiteContextHttpMethod": cookie_same_site_http_method_to_str(
            access.same_site_context_http_method,
        ),
        "schemefulSameSiteContextHttpMethod": cookie_same_site_http_method_to_str(
            access.schemeful_same_site_context_http_method,
        ),
        "sameSiteContextRedirectType": cookie_same_site_redirect_type_to_str(
            access.same_site_context_redirect_type,
        ),
        "schemefulSameSiteContextRedirectType": cookie_same_site_redirect_type_to_str(
            access.schemeful_same_site_context_redirect_type,
        ),
        "accessSemantics": cookie_access_semantics_to_str(access.access_semantics),
        "scopeSemantics": cookie_scope_semantics_to_str(access.scope_semantics),
        "isAllowedToAccessSecureCookies": access.is_allowed_to_access_secure_cookies,
        "siteForCookiesUrl": access.site_for_cookies_url.as_ref().map(Url::as_str),
        "siteForCookiesSource": cookie_browser_context_value_source_to_str(
            access.site_for_cookies_source,
        ),
        "topFrameOriginUrl": access.top_frame_origin_url.as_ref().map(Url::as_str),
        "topFrameOriginSource": cookie_browser_context_value_source_to_str(
            access.top_frame_origin_source,
        ),
        "storageAccessStatus": cookie_storage_access_status_to_str(access.storage_access_status),
        "storageAccessStatusSource": cookie_browser_context_value_source_to_str(
            access.storage_access_status_source,
        ),
        "siteContextBasis": cookie_site_context_basis_to_str(access.site_context_basis),
    })
}

fn cookie_set_status_to_json(status: &StoredCookieSetStatus) -> Value {
    match status {
        StoredCookieSetStatus::Accepted(action) => json!({
            "kind": "Accepted",
            "storeAction": format!("{action:?}"),
        }),
        StoredCookieSetStatus::Rejected(reason) => json!({
            "kind": "Rejected",
            "reason": cookie_set_rejection_reason_to_str(*reason),
        }),
    }
}

fn cookie_set_warning_reason_to_str(reason: &StoredCookieSetWarningReason) -> &'static str {
    match reason {
        StoredCookieSetWarningReason::DomainAttributeIgnored => "DomainAttributeIgnored",
        StoredCookieSetWarningReason::PathAttributeIgnored => "PathAttributeIgnored",
        StoredCookieSetWarningReason::SecureAccessGrantedNonCryptographic => {
            "SecureAccessGrantedNonCryptographic"
        }
    }
}

fn cookie_warning_reason_to_str(reason: &StoredCookieWarningReason) -> &'static str {
    match reason {
        StoredCookieWarningReason::SchemefulSameSiteContextMismatch => {
            "SchemefulSameSiteContextMismatch"
        }
        StoredCookieWarningReason::StrictLaxDowngradeStrictSameSite => {
            "StrictLaxDowngradeStrictSameSite"
        }
        StoredCookieWarningReason::StrictCrossDowngradeStrictSameSite => {
            "StrictCrossDowngradeStrictSameSite"
        }
        StoredCookieWarningReason::StrictCrossDowngradeLaxSameSite => {
            "StrictCrossDowngradeLaxSameSite"
        }
        StoredCookieWarningReason::LaxCrossDowngradeStrictSameSite => {
            "LaxCrossDowngradeStrictSameSite"
        }
        StoredCookieWarningReason::LaxCrossDowngradeLaxSameSite => "LaxCrossDowngradeLaxSameSite",
        StoredCookieWarningReason::SameSiteContextDowngradedByRedirect => {
            "SameSiteContextDowngradedByRedirect"
        }
        StoredCookieWarningReason::SecureAccessGrantedNonCryptographic => {
            "SecureAccessGrantedNonCryptographic"
        }
    }
}

fn cookie_same_site_context_downgrade_type_to_str(
    downgrade_type: StoredCookieSameSiteContextDowngradeType,
) -> &'static str {
    match downgrade_type {
        StoredCookieSameSiteContextDowngradeType::StrictToLax => "StrictToLax",
        StoredCookieSameSiteContextDowngradeType::StrictToCross => "StrictToCross",
        StoredCookieSameSiteContextDowngradeType::LaxToCross => "LaxToCross",
    }
}

fn cookie_same_site_http_method_to_str(method: StoredCookieSameSiteHttpMethod) -> &'static str {
    match method {
        StoredCookieSameSiteHttpMethod::Unset => "Unset",
        StoredCookieSameSiteHttpMethod::Unknown => "Unknown",
        StoredCookieSameSiteHttpMethod::Get => "GET",
        StoredCookieSameSiteHttpMethod::Head => "HEAD",
        StoredCookieSameSiteHttpMethod::Post => "POST",
        StoredCookieSameSiteHttpMethod::Put => "PUT",
        StoredCookieSameSiteHttpMethod::Delete => "DELETE",
        StoredCookieSameSiteHttpMethod::Connect => "CONNECT",
        StoredCookieSameSiteHttpMethod::Options => "OPTIONS",
        StoredCookieSameSiteHttpMethod::Trace => "TRACE",
        StoredCookieSameSiteHttpMethod::Patch => "PATCH",
    }
}

fn cookie_same_site_redirect_type_to_str(
    redirect_type: StoredCookieSameSiteRedirectType,
) -> &'static str {
    match redirect_type {
        StoredCookieSameSiteRedirectType::Unset => "Unset",
        StoredCookieSameSiteRedirectType::NoRedirect => "NoRedirect",
        StoredCookieSameSiteRedirectType::CrossSiteRedirect => "CrossSiteRedirect",
        StoredCookieSameSiteRedirectType::PartialSameSiteRedirect => "PartialSameSiteRedirect",
        StoredCookieSameSiteRedirectType::AllSameSiteRedirect => "AllSameSiteRedirect",
    }
}

fn cookie_request_same_site_context_to_str(
    context: StoredCookieRequestSameSiteContext,
) -> &'static str {
    match context {
        StoredCookieRequestSameSiteContext::SameSiteStrict => "SameSiteStrict",
        StoredCookieRequestSameSiteContext::SameSiteLax => "SameSiteLax",
        StoredCookieRequestSameSiteContext::SameSiteLaxMethodUnsafe => "SameSiteLaxMethodUnsafe",
        StoredCookieRequestSameSiteContext::CrossSite => "CrossSite",
    }
}

fn cookie_exclusion_reason_to_str(reason: &StoredCookieExclusionReason) -> &'static str {
    match reason {
        StoredCookieExclusionReason::CookiesDisabled => "CookiesDisabled",
        StoredCookieExclusionReason::StorageAccessBlocked => "StorageAccessBlocked",
        StoredCookieExclusionReason::StoreUnavailable => "StoreUnavailable",
        StoredCookieExclusionReason::Expired => "Expired",
        StoredCookieExclusionReason::DomainMismatch => "DomainMismatch",
        StoredCookieExclusionReason::PathMismatch => "PathMismatch",
        StoredCookieExclusionReason::SecureOnly => "SecureOnly",
        StoredCookieExclusionReason::HttpOnly => "HttpOnly",
        StoredCookieExclusionReason::PortMismatch => "PortMismatch",
        StoredCookieExclusionReason::SchemeMismatch => "SchemeMismatch",
        StoredCookieExclusionReason::SameSiteStrict => "SameSiteStrict",
        StoredCookieExclusionReason::SameSiteLax => "SameSiteLax",
        StoredCookieExclusionReason::SameSiteUnspecifiedTreatedAsLax => {
            "SameSiteUnspecifiedTreatedAsLax"
        }
        StoredCookieExclusionReason::PartitionKeyMismatch => "PartitionKeyMismatch",
    }
}

fn cdp_cookie_blocked_reason_to_str(reason: &StoredCookieExclusionReason) -> &'static str {
    // The cookie jar keeps an implementation-facing diagnostic taxonomy. CDP
    // exposes the narrower Network.CookieBlockedReason enum, including names
    // such as NotOnPath that differ from the internal reason spelling.
    match reason {
        StoredCookieExclusionReason::CookiesDisabled
        | StoredCookieExclusionReason::StorageAccessBlocked => "UserPreferences",
        StoredCookieExclusionReason::StoreUnavailable
        | StoredCookieExclusionReason::Expired
        | StoredCookieExclusionReason::HttpOnly => "UnknownError",
        StoredCookieExclusionReason::DomainMismatch => "DomainMismatch",
        StoredCookieExclusionReason::PathMismatch => "NotOnPath",
        StoredCookieExclusionReason::SecureOnly => "SecureOnly",
        StoredCookieExclusionReason::PortMismatch => "PortMismatch",
        StoredCookieExclusionReason::SchemeMismatch => "SchemeMismatch",
        StoredCookieExclusionReason::SameSiteStrict => "SameSiteStrict",
        StoredCookieExclusionReason::SameSiteLax => "SameSiteLax",
        StoredCookieExclusionReason::SameSiteUnspecifiedTreatedAsLax => {
            "SameSiteUnspecifiedTreatedAsLax"
        }
        StoredCookieExclusionReason::PartitionKeyMismatch => "UnknownError",
    }
}

fn cookie_access_semantics_to_str(semantics: StoredCookieAccessSemantics) -> &'static str {
    match semantics {
        StoredCookieAccessSemantics::Unknown => "Unknown",
        StoredCookieAccessSemantics::NonLegacy => "NonLegacy",
        StoredCookieAccessSemantics::Legacy => "Legacy",
    }
}

fn cookie_scope_semantics_to_str(semantics: StoredCookieScopeSemantics) -> &'static str {
    match semantics {
        StoredCookieScopeSemantics::Unknown => "Unknown",
        StoredCookieScopeSemantics::NonLegacy => "NonLegacy",
        StoredCookieScopeSemantics::Legacy => "Legacy",
    }
}

fn cookie_storage_access_status_to_str(status: StoredCookieStorageAccessStatus) -> &'static str {
    match status {
        StoredCookieStorageAccessStatus::None => "None",
        StoredCookieStorageAccessStatus::Granted => "Granted",
    }
}

fn cookie_browser_context_value_source_to_str(
    source: StoredCookieBrowserContextValueSource,
) -> &'static str {
    match source {
        StoredCookieBrowserContextValueSource::Unset => "Unset",
        StoredCookieBrowserContextValueSource::RequestContext => "RequestContext",
        StoredCookieBrowserContextValueSource::FacadeDefault => "FacadeDefault",
        StoredCookieBrowserContextValueSource::FacadeOverride => "FacadeOverride",
    }
}

fn cookie_site_context_basis_to_str(basis: StoredCookieSiteContextBasis) -> &'static str {
    match basis {
        StoredCookieSiteContextBasis::None => "None",
        StoredCookieSiteContextBasis::SiteForCookies => "SiteForCookies",
        StoredCookieSiteContextBasis::TopFrameOrigin => "TopFrameOrigin",
    }
}

fn cookie_set_rejection_reason_to_str(reason: StoredCookieSetRejectionReason) -> &'static str {
    match reason {
        StoredCookieSetRejectionReason::InvalidOctets => "InvalidOctets",
        StoredCookieSetRejectionReason::InvalidUrl => "InvalidUrl",
        StoredCookieSetRejectionReason::MissingCookieUrl => "MissingCookieUrl",
        StoredCookieSetRejectionReason::EmptyNameAndValue => "EmptyNameAndValue",
        StoredCookieSetRejectionReason::EmptyNameValueContainsEquals => {
            "EmptyNameValueContainsEquals"
        }
        StoredCookieSetRejectionReason::NameContainsEquals => "NameContainsEquals",
        StoredCookieSetRejectionReason::PathMustStartWithSlash => "PathMustStartWithSlash",
        StoredCookieSetRejectionReason::InvalidPartitionKey => "InvalidPartitionKey",
        StoredCookieSetRejectionReason::CookiesDisabled => "CookiesDisabled",
        StoredCookieSetRejectionReason::StorageAccessBlocked => "StorageAccessBlocked",
        StoredCookieSetRejectionReason::StoreUnavailable => "StoreUnavailable",
        StoredCookieSetRejectionReason::NonHttpScheme => "NonHttpScheme",
        StoredCookieSetRejectionReason::SecureOnly => "SecureOnly",
        StoredCookieSetRejectionReason::NonRelativeScheme => "NonRelativeScheme",
        StoredCookieSetRejectionReason::DomainMismatch => "DomainMismatch",
        StoredCookieSetRejectionReason::SameSiteNoneRequiresSecure => "SameSiteNoneRequiresSecure",
        StoredCookieSetRejectionReason::SameSiteStrict => "SameSiteStrict",
        StoredCookieSetRejectionReason::SameSiteLax => "SameSiteLax",
        StoredCookieSetRejectionReason::SameSiteUnspecifiedTreatedAsLax => {
            "SameSiteUnspecifiedTreatedAsLax"
        }
        StoredCookieSetRejectionReason::PrefixViolation => "PrefixViolation",
        StoredCookieSetRejectionReason::SecureOverlay => "SecureOverlay",
        StoredCookieSetRejectionReason::NameValueTooLarge => "NameValueTooLarge",
        StoredCookieSetRejectionReason::PartitionedMissingPartitionKey => {
            "PartitionedMissingPartitionKey"
        }
        StoredCookieSetRejectionReason::PartitionedRequiresSecure => "PartitionedRequiresSecure",
        StoredCookieSetRejectionReason::UnsupportedPartitioned => "UnsupportedPartitioned",
        StoredCookieSetRejectionReason::StorageFull => "StorageFull",
        StoredCookieSetRejectionReason::Expired => "Expired",
        StoredCookieSetRejectionReason::Parse => "Parse",
        StoredCookieSetRejectionReason::PublicSuffix => "PublicSuffix",
        StoredCookieSetRejectionReason::UnspecifiedDomain => "UnspecifiedDomain",
    }
}

fn cookie_effective_same_site_to_str(same_site: StoredCookieEffectiveSameSite) -> &'static str {
    match same_site {
        StoredCookieEffectiveSameSite::NoRestriction => "NoRestriction",
        StoredCookieEffectiveSameSite::Lax => "Lax",
        StoredCookieEffectiveSameSite::LaxAllowUnsafe => "LaxAllowUnsafe",
        StoredCookieEffectiveSameSite::Strict => "Strict",
    }
}

fn cdp_timestamp_from_offset_datetime(value: time::OffsetDateTime) -> f64 {
    value.unix_timestamp() as f64 + (value.nanosecond() as f64 / 1_000_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::{StoredCookieExclusionReason, cdp_cookie_blocked_reason_to_str};

    #[test]
    fn every_internal_cookie_exclusion_projects_to_a_public_cdp_enum_value() {
        for (reason, expected) in [
            (
                StoredCookieExclusionReason::CookiesDisabled,
                "UserPreferences",
            ),
            (
                StoredCookieExclusionReason::StorageAccessBlocked,
                "UserPreferences",
            ),
            (
                StoredCookieExclusionReason::StoreUnavailable,
                "UnknownError",
            ),
            (StoredCookieExclusionReason::Expired, "UnknownError"),
            (StoredCookieExclusionReason::HttpOnly, "UnknownError"),
            (
                StoredCookieExclusionReason::DomainMismatch,
                "DomainMismatch",
            ),
            (StoredCookieExclusionReason::PathMismatch, "NotOnPath"),
            (StoredCookieExclusionReason::SecureOnly, "SecureOnly"),
            (StoredCookieExclusionReason::PortMismatch, "PortMismatch"),
            (
                StoredCookieExclusionReason::SchemeMismatch,
                "SchemeMismatch",
            ),
            (
                StoredCookieExclusionReason::SameSiteStrict,
                "SameSiteStrict",
            ),
            (StoredCookieExclusionReason::SameSiteLax, "SameSiteLax"),
            (
                StoredCookieExclusionReason::PartitionKeyMismatch,
                "UnknownError",
            ),
        ] {
            assert_eq!(cdp_cookie_blocked_reason_to_str(&reason), expected);
        }
    }
}
