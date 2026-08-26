use cookie::SameSite;

use crate::cookie::Cookie;
use crate::utils::{is_http_scheme, is_secure, is_trustworthy_non_cryptographic};

use super::policy::{
    access_semantics, effective_same_site, scope_semantics, source_scheme_mismatch,
};
use super::*;

pub(super) fn query_context_access_result(
    cookie: &Cookie<'_>,
    context: &QueryContext<'_>,
) -> CookieAccessResult {
    let mut status = CookieInclusionStatus::default();
    let effective_same_site = effective_same_site(cookie);
    let access_semantics = access_semantics(cookie);
    let scope_semantics = scope_semantics(cookie);
    let is_allowed_to_access_secure_cookies = is_secure(context.url);

    let finish = |status: CookieInclusionStatus| CookieAccessResult {
        status,
        effective_same_site,
        same_site_context: context.same_site_context,
        same_site_context_metadata: same_site_context_metadata_for_access(
            context.same_site_context_metadata,
            context.http_method,
            context.redirect_type,
        ),
        access_semantics,
        scope_semantics,
        is_allowed_to_access_secure_cookies,
        browser_context: context.browser_context.clone(),
    };

    if cookie.is_expired() {
        status.add_exclusion(CookieExclusionReason::Expired);
    }
    if !cookie.domain.matches(context.url) {
        status.add_exclusion(CookieExclusionReason::DomainMismatch);
    }
    if !cookie.path.matches(context.url) {
        status.add_exclusion(CookieExclusionReason::PathMismatch);
    }
    if cookie.secure().unwrap_or(false) && !is_secure(context.url) {
        status.add_exclusion(CookieExclusionReason::SecureOnly);
    }
    if cookie.secure().unwrap_or(false) && is_trustworthy_non_cryptographic(context.url) {
        // Chromium emits an advisory warning for localhost/loopback-style
        // secure access instead of silently treating it as plain HTTPS. Keep
        // the same separation here so diagnostics can distinguish
        // "cryptographic" from "trustworthy non-cryptographic".
        status.add_warning(CookieWarningReason::SecureAccessGrantedNonCryptographic);
    }

    let allow_http_only =
        context.include_http_only || context.source != CookieAccessSource::Document;
    if cookie.http_only().unwrap_or(false) && !allow_http_only {
        status.add_exclusion(CookieExclusionReason::HttpOnly);
    }
    if cookie.http_only().unwrap_or(false)
        && context.source == CookieAccessSource::Http
        && !is_http_scheme(context.url)
    {
        status.add_exclusion(CookieExclusionReason::HttpOnly);
    }
    if source_scheme_mismatch(cookie, context.url) {
        status.add_exclusion(CookieExclusionReason::SchemeMismatch);
    }
    let partition_key_mismatch = match (
        cookie.partition_key(),
        context.browser_context.cookie_partition_key.as_ref(),
    ) {
        (Some(cookie_key), Some(context_key)) => cookie_key != context_key,
        (Some(_), None) => true,
        (None, Some(crate::CookiePartitionKey::Opaque { .. })) => true,
        (None, Some(crate::CookiePartitionKey::Site { .. }) | None) => false,
    };
    if partition_key_mismatch {
        status.add_exclusion(CookieExclusionReason::PartitionKeyMismatch);
    }

    // SameSite gates request inclusion, not raw cookie visibility. Keep the
    // first version scoped to network reads so `document.cookie` and browser
    // introspection continue to expose the stored value while request paths can
    // start modeling cross-site exclusions explicitly. Unspecified SameSite is
    // intentionally left alone here until the core grows a fuller
    // navigation/method-aware lax-by-default model. Deliberately keep
    // accumulating exclusion reasons instead of returning early: Chromium's
    // `CookieInclusionStatus` is a set, and higher layers should be able to
    // inspect "why else would this cookie have been blocked" without changing
    // the first-reason compatibility projection.
    let schemeful_same_site =
        same_site_exclusion_reason(cookie, context, context.same_site_context.for_inclusion());
    let schemeless_same_site =
        same_site_exclusion_reason(cookie, context, context.same_site_context.context);

    if schemeful_same_site != schemeless_same_site {
        // Chromium classifies schemeful-context breakage more precisely than a
        // single generic warning. Preserve the downgrade shape when the
        // metadata is available, and only fall back to the coarse mismatch
        // reason when the wrapper has not populated a stable downgrade type.
        status.add_warning(
            schemeful_same_site_mismatch_warning(
                effective_same_site,
                context
                    .same_site_context_metadata
                    .schemeful_context
                    .downgrade_type
                    .or_else(|| infer_schemeful_same_site_downgrade(context.same_site_context)),
            )
            .unwrap_or(CookieWarningReason::SchemefulSameSiteContextMismatch),
        );
    }

    if let Some(reason) = schemeful_same_site {
        if context
            .same_site_context_metadata
            .schemeful_context
            .downgraded_by_cross_site_redirect
        {
            // Keep redirect-downgrade diagnostics tied to requests that are
            // actually SameSite-blocked, otherwise every cross-site redirect
            // would generate low-signal noise even for unrestricted cookies.
            status.add_warning(CookieWarningReason::SameSiteContextDowngradedByRedirect);
        }
        status.add_exclusion(reason);
    }

    finish(status)
}

pub(super) fn same_site_context_metadata_for_access(
    metadata: SameSiteContextMetadata,
    http_method: SameSiteContextHttpMethod,
    redirect_type: SameSiteContextRedirectType,
) -> SameSiteContextMetadata {
    SameSiteContextMetadata::new(
        metadata
            .context
            .with_http_method(http_method)
            .with_redirect_type(redirect_type),
        metadata
            .schemeful_context
            .with_http_method(http_method)
            .with_redirect_type(redirect_type),
    )
}

pub(super) fn same_site_exclusion_reason(
    cookie: &Cookie<'_>,
    context: &QueryContext<'_>,
    same_site_context: SameSiteRequestContext,
) -> Option<CookieExclusionReason> {
    if context.source != CookieAccessSource::Http {
        return None;
    }

    match cookie.same_site() {
        Some(SameSite::Strict) if same_site_context != SameSiteRequestContext::SameSiteStrict => {
            return Some(CookieExclusionReason::SameSiteStrict);
        }
        // Explicit `SameSite=Lax` cookies stay excluded in the
        // `LaxMethodUnsafe` context. Chromium's additional
        // "Lax-allow-unsafe" carve-out only applies to cookies that did not
        // opt into `SameSite=Lax` explicitly.
        Some(SameSite::Lax)
            if !matches!(
                same_site_context,
                SameSiteRequestContext::SameSiteStrict | SameSiteRequestContext::SameSiteLax
            ) =>
        {
            return Some(CookieExclusionReason::SameSiteLax);
        }
        _ => {}
    }

    None
}

pub(super) fn schemeful_same_site_mismatch_warning(
    effective_same_site: CookieEffectiveSameSite,
    downgrade_type: Option<SameSiteContextDowngradeType>,
) -> Option<CookieWarningReason> {
    match (downgrade_type, effective_same_site) {
        (Some(SameSiteContextDowngradeType::StrictToLax), CookieEffectiveSameSite::Strict) => {
            Some(CookieWarningReason::StrictLaxDowngradeStrictSameSite)
        }
        (Some(SameSiteContextDowngradeType::StrictToCross), CookieEffectiveSameSite::Strict) => {
            Some(CookieWarningReason::StrictCrossDowngradeStrictSameSite)
        }
        (Some(SameSiteContextDowngradeType::StrictToCross), CookieEffectiveSameSite::Lax) => {
            Some(CookieWarningReason::StrictCrossDowngradeLaxSameSite)
        }
        (Some(SameSiteContextDowngradeType::LaxToCross), CookieEffectiveSameSite::Strict) => {
            Some(CookieWarningReason::LaxCrossDowngradeStrictSameSite)
        }
        (Some(SameSiteContextDowngradeType::LaxToCross), CookieEffectiveSameSite::Lax) => {
            Some(CookieWarningReason::LaxCrossDowngradeLaxSameSite)
        }
        _ => None,
    }
}

pub(super) fn infer_schemeful_same_site_downgrade(
    same_site_context: SameSiteContext,
) -> Option<SameSiteContextDowngradeType> {
    match (
        same_site_context.context,
        same_site_context.schemeful_context,
    ) {
        (SameSiteRequestContext::SameSiteStrict, SameSiteRequestContext::SameSiteLax) => {
            Some(SameSiteContextDowngradeType::StrictToLax)
        }
        (
            SameSiteRequestContext::SameSiteStrict,
            SameSiteRequestContext::CrossSite | SameSiteRequestContext::SameSiteLaxMethodUnsafe,
        ) => Some(SameSiteContextDowngradeType::StrictToCross),
        (
            SameSiteRequestContext::SameSiteLax,
            SameSiteRequestContext::CrossSite | SameSiteRequestContext::SameSiteLaxMethodUnsafe,
        ) => Some(SameSiteContextDowngradeType::LaxToCross),
        _ => None,
    }
}
