use cookie::SameSite;
use url::Url;

use crate::cookie::Cookie;
use crate::utils::is_secure;
use crate::CookieError;

use super::policy::{cookie_name_value_too_large, prefixes_are_valid};
use super::*;

pub(super) fn should_downgrade_identical_ip_domain_to_host_only(
    cookie: &crate::Cookie<'_>,
    request_url: &Url,
) -> bool {
    matches!(
        request_url.host(),
        Some(url::Host::Ipv4(_)) | Some(url::Host::Ipv6(_))
    ) && matches!(cookie.domain, crate::CookieDomain::Suffix(_))
        && cookie.domain.host_is_identical(request_url)
}

pub(super) fn canonicalize_cookie_for_store_checks<'a>(
    store: &CookieStore,
    mut cookie: Cookie<'a>,
    request_url: &Url,
) -> Result<Cookie<'a>, CookieError> {
    #[cfg(feature = "public_suffix")]
    if let Some(ref psl) = store.public_suffix_list {
        if cookie.domain.is_public_suffix(psl.as_ref()) {
            if cookie.domain.host_is_identical(request_url) {
                cookie.domain = crate::cookie_domain::CookieDomain::host_only(request_url)?;
            } else {
                return Err(CookieError::PublicSuffix);
            }
        }
    }
    if should_downgrade_identical_ip_domain_to_host_only(&cookie, request_url) {
        cookie.domain = crate::cookie_domain::CookieDomain::host_only(request_url)?;
    }

    Ok(cookie)
}

pub(super) fn collect_preinsert_rejection_reasons(
    store: &CookieStore,
    cookie: &Cookie<'_>,
    context: &InsertContext<'_>,
) -> Vec<CookieSetRejectionReason> {
    let mut reasons = Vec::new();
    let request_url = context.url;

    if cookie.http_only().unwrap_or(false) && context.source == CookieAccessSource::Document {
        reasons.push(CookieSetRejectionReason::NonHttpScheme);
    }
    if context.enforce_browser_policy && cookie.secure().unwrap_or(false) && !is_secure(request_url)
    {
        reasons.push(CookieSetRejectionReason::SecureOnly);
    }
    if context.enforce_browser_policy
        && cookie.same_site() == Some(SameSite::None)
        && !cookie.secure().unwrap_or(false)
    {
        reasons.push(CookieSetRejectionReason::SameSiteNoneRequiresSecure);
    }
    if context.enforce_browser_policy && cookie_name_value_too_large(cookie.name(), cookie.value())
    {
        reasons.push(CookieSetRejectionReason::NameValueTooLarge);
    }
    if cookie.partitioned().unwrap_or(false) {
        if context.enforce_browser_policy && !cookie.secure().unwrap_or(false) {
            reasons.push(CookieSetRejectionReason::PartitionedRequiresSecure);
        }
        if cookie.partition_key().is_none() {
            reasons.push(CookieSetRejectionReason::PartitionedMissingPartitionKey);
        }
    }

    let cookie = match canonicalize_cookie_for_store_checks(store, cookie.clone(), request_url) {
        Ok(cookie) => cookie,
        Err(error) => {
            reasons.push(error.into());
            return reasons;
        }
    };

    if context.enforce_browser_policy && !prefixes_are_valid(&cookie) {
        reasons.push(CookieSetRejectionReason::PrefixViolation);
    }
    if !cookie.domain.matches(request_url) {
        reasons.push(CookieSetRejectionReason::DomainMismatch);
    }
    if context.enforce_browser_policy
        && context.source != CookieAccessSource::Cdp
        && !cookie.secure().unwrap_or(false)
        && !is_secure(request_url)
        && store.has_secure_overlay_conflict(&cookie)
    {
        reasons.push(CookieSetRejectionReason::SecureOverlay);
    }
    if let Some(cookie_domain) = cookie.domain.as_cow() {
        if store
            .get_with_partition_key(
                &cookie_domain,
                &cookie.path,
                cookie.name(),
                cookie.partition_key(),
            )
            .is_some_and(|old_cookie| {
                old_cookie.http_only().unwrap_or(false)
                    && context.source == CookieAccessSource::Document
            })
        {
            reasons.push(CookieSetRejectionReason::NonHttpScheme);
        }
    } else {
        reasons.push(CookieSetRejectionReason::UnspecifiedDomain);
    }

    reasons
}

pub(super) fn provisional_set_access_result() -> CookieSetAccessResult {
    CookieSetAccessResult {
        status: CookieSetResult::Accepted(StoreAction::Inserted),
        rejection_reasons: Vec::new(),
        warning_reasons: Vec::new(),
        effective_same_site: None,
    }
}

pub(super) fn rejected_set_access_result(
    reason: CookieSetRejectionReason,
) -> CookieSetAccessResult {
    CookieSetAccessResult {
        status: CookieSetResult::Rejected(reason),
        rejection_reasons: vec![reason],
        warning_reasons: Vec::new(),
        effective_same_site: None,
    }
}

pub(super) fn merge_rejected_set_access_result(
    mut prior_result: CookieSetAccessResult,
    reason: CookieSetRejectionReason,
) -> CookieSetAccessResult {
    prior_result.add_rejection(reason);
    prior_result
}
