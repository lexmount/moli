use crate::integrity::integrity_metadata_hashes;
use crate::policy::{ContentSecurityPolicy, directive_source_list};
use crate::reporting::{
    content_security_policy_report_to_endpoints, content_security_policy_report_uri_endpoints,
    csp_url_for_report,
};
use crate::types::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_crypto::DigestAlgorithm;
use percent_encoding::percent_decode_str;
use url::Url;

const CONNECT_SRC: &str = "connect-src";
const CHILD_SRC: &str = "child-src";
const DEFAULT_SRC: &str = "default-src";
const BASE_URI: &str = "base-uri";
const FRAME_ANCESTORS: &str = "frame-ancestors";
const FRAME_SRC: &str = "frame-src";
const FONT_SRC: &str = "font-src";
const IMG_SRC: &str = "img-src";
const MANIFEST_SRC: &str = "manifest-src";
const MEDIA_SRC: &str = "media-src";
const SCRIPT_SRC_ATTR: &str = "script-src-attr";
const SCRIPT_SRC_ELEM: &str = "script-src-elem";
const SCRIPT_SRC: &str = "script-src";
const STYLE_SRC_ATTR: &str = "style-src-attr";
const STYLE_SRC_ELEM: &str = "style-src-elem";
const STYLE_SRC: &str = "style-src";
const WORKER_SRC: &str = "worker-src";
const REQUIRE_TRUSTED_TYPES_FOR: &str = "require-trusted-types-for";
const TRUSTED_TYPES: &str = "trusted-types";
const SANDBOX: &str = "sandbox";

pub fn content_security_policy_requires_trusted_types_for_script(
    policies: &[ContentSecurityPolicy],
) -> bool {
    policies
        .iter()
        .any(policy_requires_trusted_types_for_script)
}

pub fn content_security_policy_allows_trusted_types_eval(
    policies: &[ContentSecurityPolicy],
) -> bool {
    // Like Blink's ContentSecurityPolicy::AllowTrustedTypesEval, this only
    // activates the document-global Trusted Types relaxation. It is not the
    // final CSP decision: CSP3 EnsureCSPDoesNotBlockStringCompilation still
    // evaluates every policy through the TrustedTypesEval non-URL gate.
    policies
        .iter()
        .any(policy_requires_trusted_types_for_script)
        && policies.iter().any(policy_allows_trusted_types_eval)
}

pub fn content_security_policy_allows_trusted_type_policy_name(
    policies: &[ContentSecurityPolicy],
    policy_name: &str,
) -> bool {
    policies
        .iter()
        .all(|policy| policy_allows_trusted_type_policy_name(policy, policy_name, false))
}

pub fn content_security_policy_sandboxes_document_domain(
    policies: &[ContentSecurityPolicy],
) -> bool {
    policies.iter().any(policy_sandboxes_document_domain)
}

pub fn content_security_policy_forces_opaque_origin(policies: &[ContentSecurityPolicy]) -> bool {
    policies.iter().any(policy_forces_opaque_origin)
}

pub fn content_security_policy_sandbox_allows_scripts(policies: &[ContentSecurityPolicy]) -> bool {
    policies
        .iter()
        .all(|policy| policy_sandbox_allows_scripts(policy).unwrap_or(true))
}

pub fn content_security_policy_sandbox_allows_modals(policies: &[ContentSecurityPolicy]) -> bool {
    policies
        .iter()
        .all(|policy| policy_sandbox_allows_modals(policy).unwrap_or(true))
}

pub fn content_security_policy_sandbox_allows_top_navigation(
    policies: &[ContentSecurityPolicy],
    user_activated: bool,
) -> bool {
    policies.iter().all(|policy| {
        directive_source_list(policy, SANDBOX).is_none_or(|sources| {
            sources.iter().any(|token| {
                token.eq_ignore_ascii_case("allow-top-navigation")
                    || (user_activated
                        && token.eq_ignore_ascii_case("allow-top-navigation-by-user-activation"))
            })
        })
    })
}

pub fn content_security_policy_sandbox_allows_popups_to_escape(
    policies: &[ContentSecurityPolicy],
) -> bool {
    let mut has_sandbox = false;
    for policy in policies {
        let Some(allows) = policy_sandbox_allows_popups_to_escape(policy) else {
            continue;
        };
        has_sandbox = true;
        if !allows {
            return false;
        }
    }
    has_sandbox
}

pub fn ensure_content_security_policy_allows_url(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    error: impl FnOnce() -> String,
) -> Result<(), String> {
    ensure_content_security_policy_allows_url_with_redirect_status(
        policies,
        protected_url,
        request_url,
        kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
        error,
    )
}

pub fn ensure_content_security_policy_allows_url_with_redirect_status(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    error: impl FnOnce() -> String,
) -> Result<(), String> {
    if content_security_policy_allows_url_with_redirect_status(
        policies,
        protected_url,
        request_url,
        kind,
        redirect_status,
    ) {
        Ok(())
    } else {
        Err(error())
    }
}

#[cfg(test)]
fn content_security_policy_allows_url(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
) -> bool {
    content_security_policy_allows_url_with_redirect_status(
        policies,
        protected_url,
        request_url,
        kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
}

pub fn content_security_policy_allows_url_with_redirect_status(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> bool {
    policies.iter().all(|policy| {
        let Some((_, source_list)) = effective_source_list_with_directive(policy, kind) else {
            return true;
        };
        source_list_allows(source_list, protected_url, request_url, redirect_status)
    })
}

#[cfg(test)]
fn content_security_policy_url_violation_with_redirect_status(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_with_redirect_status_and_disposition(
        policies,
        protected_url,
        request_url,
        kind,
        redirect_status,
        ContentSecurityPolicyDisposition::Enforce,
    )
}

#[cfg(test)]
fn content_security_policy_url_violation_with_redirect_status_and_disposition(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
        policies,
        protected_url,
        request_url,
        kind,
        redirect_status,
        disposition,
        &ContentSecurityPolicyReportingEndpoints::default(),
    )
}

pub fn content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
        policies,
        protected_url,
        request_url,
        request_url,
        kind,
        redirect_status,
        disposition,
        reporting_endpoints,
    )
}

pub fn content_security_policy_frame_ancestors_violation_with_disposition_and_reporting_endpoints(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    ancestor_origins: &[Option<Url>],
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    policies.iter().find_map(|policy| {
        let source_list = normalized_source_list(directive_source_list(policy, FRAME_ANCESTORS)?);
        if ancestor_origins.iter().all(|ancestor_origin| {
            ancestor_origin.as_ref().is_some_and(|ancestor_origin| {
                frame_ancestor_source_list_allows(&source_list, protected_url, ancestor_origin)
            })
        }) {
            return None;
        }
        let document_uri = csp_url_for_report(protected_url);
        Some(ContentSecurityPolicyUrlViolation {
            effective_directive: FRAME_ANCESTORS,
            blocked_uri: document_uri.clone(),
            source_file: document_uri.clone(),
            document_uri,
            original_policy: policy.as_str().to_owned(),
            disposition,
            report_uri_endpoints: content_security_policy_report_uri_endpoints(
                policy,
                protected_url,
            ),
            report_to_endpoints: content_security_policy_report_to_endpoints(
                policy,
                reporting_endpoints,
            ),
            sample: String::new(),
            line_number: 0,
            column_number: 0,
        })
    })
}

#[allow(clippy::too_many_arguments)]
pub fn content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_reporting_endpoints_and_request(
        policies,
        protected_url,
        request_url,
        request_url,
        ContentSecurityPolicyResourceKind::DocumentScriptElement,
        redirect_status,
        disposition,
        reporting_endpoints,
        ContentSecurityPolicyUrlRequest::Script(request),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn content_security_policy_style_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    request: ContentSecurityPolicyStyleElementRequest<'_>,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_reporting_endpoints_and_request(
        policies,
        protected_url,
        request_url,
        request_url,
        ContentSecurityPolicyResourceKind::DocumentStyleElement,
        redirect_status,
        disposition,
        reporting_endpoints,
        ContentSecurityPolicyUrlRequest::Style(request),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    checked_url: &Url,
    blocked_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_reporting_endpoints_and_request(
        policies,
        protected_url,
        checked_url,
        blocked_url,
        kind,
        redirect_status,
        disposition,
        reporting_endpoints,
        ContentSecurityPolicyUrlRequest::Standard,
    )
}

#[derive(Clone, Copy)]
enum ContentSecurityPolicyUrlRequest<'a> {
    Standard,
    Script(ContentSecurityPolicyScriptElementRequest<'a>),
    Style(ContentSecurityPolicyStyleElementRequest<'a>),
}

#[allow(clippy::too_many_arguments)]
fn content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_reporting_endpoints_and_request(
    policies: &[ContentSecurityPolicy],
    protected_url: &Url,
    checked_url: &Url,
    blocked_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    request: ContentSecurityPolicyUrlRequest<'_>,
) -> Option<ContentSecurityPolicyUrlViolation> {
    policies.iter().find_map(|policy| {
        let (effective_directive, source_list) =
            effective_source_list_with_directive(policy, kind)?;
        if source_list_allows_resource_request(
            source_list,
            protected_url,
            checked_url,
            redirect_status,
            request,
        ) {
            return None;
        }
        let document_uri = csp_url_for_report(protected_url);
        Some(ContentSecurityPolicyUrlViolation {
            effective_directive,
            blocked_uri: csp_url_for_report(blocked_url),
            source_file: document_uri.clone(),
            document_uri,
            original_policy: policy.as_str().to_owned(),
            disposition,
            report_uri_endpoints: content_security_policy_report_uri_endpoints(
                policy,
                protected_url,
            ),
            report_to_endpoints: content_security_policy_report_to_endpoints(
                policy,
                reporting_endpoints,
            ),
            sample: String::new(),
            line_number: 0,
            column_number: 0,
        })
    })
}

pub fn content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints(
    policy: &ContentSecurityPolicy,
    protected_url: &Url,
    sink: &str,
    sample: &str,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    if !policy_requires_trusted_types_for_script(policy) {
        return None;
    }
    let document_uri = csp_url_for_report(protected_url);
    Some(ContentSecurityPolicyUrlViolation {
        effective_directive: REQUIRE_TRUSTED_TYPES_FOR,
        blocked_uri: "trusted-types-sink".to_owned(),
        source_file: document_uri.clone(),
        document_uri,
        original_policy: policy.as_str().to_owned(),
        disposition,
        report_uri_endpoints: content_security_policy_report_uri_endpoints(policy, protected_url),
        report_to_endpoints: content_security_policy_report_to_endpoints(
            policy,
            reporting_endpoints,
        ),
        sample: trusted_types_sink_violation_sample(sink, sample),
        line_number: 0,
        column_number: 0,
    })
}

pub fn content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
    policy: &ContentSecurityPolicy,
    protected_url: &Url,
    kind: ContentSecurityPolicyNonUrlKind,
    source: &str,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    debug_assert!(matches!(
        kind,
        ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler
            | ContentSecurityPolicyNonUrlKind::DocumentInlineNavigation
            | ContentSecurityPolicyNonUrlKind::DocumentInlineStyleAttribute
    ));
    content_security_policy_non_url_violation_with_source(
        policy,
        protected_url,
        kind,
        Some(source),
        disposition,
        reporting_endpoints,
    )
}

pub fn content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
    policy: &ContentSecurityPolicy,
    protected_url: &Url,
    source: &str,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    let kind = ContentSecurityPolicyNonUrlKind::DocumentInlineScript;
    let (effective_directive, source_list) =
        kind.directive_fallbacks().iter().find_map(|directive| {
            directive_source_list(policy, directive)
                .map(|sources| (kind.effective_directive(), sources))
        })?;
    if inline_script_element_source_list_allows(source_list.clone(), source, request) {
        return None;
    }
    let document_uri = csp_url_for_report(protected_url);
    Some(ContentSecurityPolicyUrlViolation {
        effective_directive,
        blocked_uri: kind.blocked_uri().to_owned(),
        source_file: document_uri.clone(),
        document_uri,
        original_policy: policy.as_str().to_owned(),
        disposition,
        report_uri_endpoints: content_security_policy_report_uri_endpoints(policy, protected_url),
        report_to_endpoints: content_security_policy_report_to_endpoints(
            policy,
            reporting_endpoints,
        ),
        sample: inline_source_violation_sample(&source_list, source),
        line_number: 0,
        column_number: 0,
    })
}

pub fn content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
    policy: &ContentSecurityPolicy,
    protected_url: &Url,
    source: &str,
    request: ContentSecurityPolicyStyleElementRequest<'_>,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    let kind = ContentSecurityPolicyNonUrlKind::DocumentInlineStyleElement;
    let (effective_directive, source_list) =
        kind.directive_fallbacks().iter().find_map(|directive| {
            directive_source_list(policy, directive)
                .map(|sources| (kind.effective_directive(), sources))
        })?;
    if inline_style_element_source_list_allows(source_list.clone(), source, request) {
        return None;
    }
    let document_uri = csp_url_for_report(protected_url);
    Some(ContentSecurityPolicyUrlViolation {
        effective_directive,
        blocked_uri: kind.blocked_uri().to_owned(),
        source_file: document_uri.clone(),
        document_uri,
        original_policy: policy.as_str().to_owned(),
        disposition,
        report_uri_endpoints: content_security_policy_report_uri_endpoints(policy, protected_url),
        report_to_endpoints: content_security_policy_report_to_endpoints(
            policy,
            reporting_endpoints,
        ),
        sample: inline_source_violation_sample(&source_list, source),
        line_number: 0,
        column_number: 0,
    })
}

pub fn content_security_policy_non_url_violation_with_source(
    policy: &ContentSecurityPolicy,
    protected_url: &Url,
    kind: ContentSecurityPolicyNonUrlKind,
    source: Option<&str>,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    let (effective_directive, source_list) =
        kind.directive_fallbacks().iter().find_map(|directive| {
            directive_source_list(policy, directive)
                .map(|sources| (kind.effective_directive(), sources))
        })?;
    if kind.source_list_allows(&source_list, source) {
        return None;
    }
    let document_uri = csp_url_for_report(protected_url);
    Some(ContentSecurityPolicyUrlViolation {
        effective_directive,
        blocked_uri: kind.blocked_uri().to_owned(),
        source_file: document_uri.clone(),
        document_uri,
        original_policy: policy.as_str().to_owned(),
        disposition,
        report_uri_endpoints: content_security_policy_report_uri_endpoints(policy, protected_url),
        report_to_endpoints: content_security_policy_report_to_endpoints(
            policy,
            reporting_endpoints,
        ),
        sample: source
            .map(|source| inline_source_violation_sample(&source_list, source))
            .unwrap_or_default(),
        line_number: 0,
        column_number: 0,
    })
}

fn policy_requires_trusted_types_for_script(policy: &ContentSecurityPolicy) -> bool {
    directive_source_list(policy, REQUIRE_TRUSTED_TYPES_FOR).is_some_and(|sources| {
        sources
            .iter()
            .any(|source| csp_keyword_eq(source, "script"))
    })
}

fn policy_allows_trusted_types_eval(policy: &ContentSecurityPolicy) -> bool {
    [SCRIPT_SRC, DEFAULT_SRC]
        .into_iter()
        .find_map(|directive| directive_source_list(policy, directive))
        .is_some_and(|sources| {
            sources
                .iter()
                .any(|source| csp_keyword_eq(source, "trusted-types-eval"))
        })
}

fn policy_allows_trusted_type_policy_name(
    policy: &ContentSecurityPolicy,
    policy_name: &str,
    is_duplicate: bool,
) -> bool {
    let Some(sources) = directive_source_list(policy, TRUSTED_TYPES) else {
        return true;
    };
    let name_is_allowed = sources.iter().any(|source| {
        let source = *source;
        source == "*"
            || (!source.is_empty()
                && source == policy_name
                && source.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(byte, b'-' | b'#' | b'=' | b'_' | b'/' | b'@' | b'.' | b'%')
                }))
    });
    let duplicate_is_allowed = !is_duplicate
        || sources
            .iter()
            .any(|source| csp_keyword_eq(source, "allow-duplicates"));
    name_is_allowed && duplicate_is_allowed
}

fn policy_sandboxes_document_domain(policy: &ContentSecurityPolicy) -> bool {
    directive_source_list(policy, SANDBOX).is_some()
}

fn policy_forces_opaque_origin(policy: &ContentSecurityPolicy) -> bool {
    directive_source_list(policy, SANDBOX)
        .is_some_and(|sources| !sandbox_sources_allow_same_origin(&sources))
}

fn policy_sandbox_allows_scripts(policy: &ContentSecurityPolicy) -> Option<bool> {
    directive_source_list(policy, SANDBOX).map(|sources| sandbox_sources_allow_scripts(&sources))
}

fn policy_sandbox_allows_modals(policy: &ContentSecurityPolicy) -> Option<bool> {
    directive_source_list(policy, SANDBOX).map(|sources| sandbox_sources_allow_modals(&sources))
}

fn policy_sandbox_allows_popups_to_escape(policy: &ContentSecurityPolicy) -> Option<bool> {
    directive_source_list(policy, SANDBOX)
        .map(|sources| sandbox_sources_allow_popups_to_escape(&sources))
}

fn sandbox_sources_allow_same_origin(sources: &[&str]) -> bool {
    sources
        .iter()
        .any(|token| token.eq_ignore_ascii_case("allow-same-origin"))
}

fn sandbox_sources_allow_scripts(sources: &[&str]) -> bool {
    sources
        .iter()
        .any(|token| token.eq_ignore_ascii_case("allow-scripts"))
}

fn sandbox_sources_allow_modals(sources: &[&str]) -> bool {
    sources
        .iter()
        .any(|token| token.eq_ignore_ascii_case("allow-modals"))
}

fn sandbox_sources_allow_popups_to_escape(sources: &[&str]) -> bool {
    sources
        .iter()
        .any(|token| token.eq_ignore_ascii_case("allow-popups-to-escape-sandbox"))
}

fn trusted_types_sink_violation_sample(sink: &str, sample: &str) -> String {
    let clipped = trusted_types_violation_sample(sample);
    format!("{sink}|{clipped}")
}

fn effective_source_list_with_directive(
    policy: &ContentSecurityPolicy,
    kind: ContentSecurityPolicyResourceKind,
) -> Option<(&'static str, Vec<&str>)> {
    kind.directive_fallbacks().iter().find_map(|directive| {
        directive_source_list(policy, directive)
            .map(|sources| (kind.effective_directive(), sources))
    })
}

fn source_list_allows(
    sources: Vec<&str>,
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> bool {
    let sources = normalized_source_list(sources);
    normalized_source_list_allows_url(&sources, protected_url, request_url, redirect_status)
}

fn frame_ancestor_source_list_allows(
    sources: &[&str],
    protected_url: &Url,
    ancestor_origin: &Url,
) -> bool {
    if sources.is_empty() || (sources.len() == 1 && csp_keyword_eq(sources[0], "none")) {
        return false;
    }
    sources.iter().any(|source| {
        !csp_keyword_eq(source, "none")
            && !source_has_path(source)
            && source_expression_matches(
                source,
                protected_url,
                ancestor_origin,
                ContentSecurityPolicyRedirectStatus::NoRedirect,
            )
    })
}

fn source_list_allows_script_element_request(
    sources: Vec<&str>,
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
) -> bool {
    let sources = normalized_source_list(sources);
    if let Some(nonce) = request
        .nonce
        .map(str::trim)
        .filter(|nonce| !nonce.is_empty())
        && sources
            .iter()
            .any(|source| csp_nonce_source_matches(source, nonce))
    {
        return true;
    }
    if script_integrity_matches_hash_source(request.integrity, &sources) {
        return true;
    }
    if source_list_activates_strict_dynamic(&sources) {
        return !request.parser_inserted;
    }
    normalized_source_list_allows_url(&sources, protected_url, request_url, redirect_status)
}

fn source_list_allows_style_element_request(
    sources: Vec<&str>,
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    request: ContentSecurityPolicyStyleElementRequest<'_>,
) -> bool {
    let sources = normalized_source_list(sources);
    if let Some(nonce) = request
        .nonce
        .map(str::trim)
        .filter(|nonce| !nonce.is_empty())
        && sources
            .iter()
            .any(|source| csp_nonce_source_matches(source, nonce))
    {
        return true;
    }
    normalized_source_list_allows_url(&sources, protected_url, request_url, redirect_status)
}

fn source_list_allows_resource_request(
    sources: Vec<&str>,
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    request: ContentSecurityPolicyUrlRequest<'_>,
) -> bool {
    match request {
        ContentSecurityPolicyUrlRequest::Standard => source_list_allows_script_element_request(
            sources,
            protected_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyScriptElementRequest::default(),
        ),
        ContentSecurityPolicyUrlRequest::Script(request) => {
            source_list_allows_script_element_request(
                sources,
                protected_url,
                request_url,
                redirect_status,
                request,
            )
        }
        ContentSecurityPolicyUrlRequest::Style(request) => {
            source_list_allows_style_element_request(
                sources,
                protected_url,
                request_url,
                redirect_status,
                request,
            )
        }
    }
}

fn inline_script_element_source_list_allows(
    sources: Vec<&str>,
    source: &str,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
) -> bool {
    let sources = normalized_source_list(sources);
    if let Some(nonce) = request
        .nonce
        .map(str::trim)
        .filter(|nonce| !nonce.is_empty())
        && sources
            .iter()
            .any(|source| csp_nonce_source_matches(source, nonce))
    {
        return true;
    }
    if sources
        .iter()
        .filter_map(|source| csp_hash_source_value(source))
        .any(|hash_source| inline_source_matches_hash(source, hash_source))
    {
        return true;
    }
    let has_strict_dynamic = sources
        .iter()
        .any(|source| csp_keyword_eq(source, "strict-dynamic"));
    if has_strict_dynamic && !request.parser_inserted {
        return true;
    }
    let has_nonce_or_hash = sources.iter().any(|source| {
        csp_nonce_source_value(source).is_some() || csp_hash_source_value(source).is_some()
    });
    !has_nonce_or_hash
        && !has_strict_dynamic
        && sources
            .iter()
            .any(|source| csp_keyword_eq(source, "unsafe-inline"))
}

fn inline_style_element_source_list_allows(
    sources: Vec<&str>,
    source: &str,
    request: ContentSecurityPolicyStyleElementRequest<'_>,
) -> bool {
    let sources = normalized_source_list(sources);
    if let Some(nonce) = request
        .nonce
        .map(str::trim)
        .filter(|nonce| !nonce.is_empty())
        && sources
            .iter()
            .any(|source| csp_nonce_source_matches(source, nonce))
    {
        return true;
    }
    if sources
        .iter()
        .filter_map(|source| csp_hash_source_value(source))
        .any(|hash_source| inline_source_matches_hash(source, hash_source))
    {
        return true;
    }
    let has_nonce_or_hash = sources.iter().any(|source| {
        csp_nonce_source_value(source).is_some() || csp_hash_source_value(source).is_some()
    });
    !has_nonce_or_hash
        && sources
            .iter()
            .any(|source| csp_keyword_eq(source, "unsafe-inline"))
}

fn inline_source_violation_sample(source_list: &[&str], source: &str) -> String {
    if !source_list
        .iter()
        .any(|source| csp_keyword_eq(source, "report-sample"))
    {
        return String::new();
    }
    source.chars().take(40).collect()
}

fn normalized_source_list(sources: Vec<&str>) -> Vec<&str> {
    // The directive parser already splits on CSP's ASCII whitespace. Trimming
    // again would turn invalid tokens containing vertical tabs into sources.
    sources
        .into_iter()
        .filter(|source| !source.is_empty())
        .collect()
}

fn normalized_source_list_allows_url(
    sources: &[&str],
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> bool {
    if sources.is_empty() {
        return false;
    }
    if sources.len() == 1 && csp_keyword_eq(sources[0], "none") {
        return false;
    }
    sources.iter().any(|source| {
        !csp_keyword_eq(source, "none")
            && source_expression_matches(source, protected_url, request_url, redirect_status)
    })
}

fn csp_nonce_source_matches(source: &str, nonce: &str) -> bool {
    csp_nonce_source_value(source).is_some_and(|value| value == nonce)
}

fn source_list_activates_strict_dynamic(sources: &[&str]) -> bool {
    sources
        .iter()
        .any(|source| csp_keyword_eq(source, "strict-dynamic"))
        && sources.iter().any(|source| {
            csp_nonce_source_value(source).is_some() || csp_hash_source_value(source).is_some()
        })
}

fn csp_nonce_source_value(source: &str) -> Option<&str> {
    if !source.get(..7)?.eq_ignore_ascii_case("'nonce-") {
        return None;
    }
    let value = source.get(7..)?.strip_suffix('\'')?;
    let digits = value.trim_end_matches('=');
    (!digits.is_empty()
        && value.len() - digits.len() <= 2
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'-' | b'_')))
    .then_some(value)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
enum CspHashAlgorithm {
    Sha256,
    Sha384,
    Sha512,
}

impl CspHashAlgorithm {
    fn digest_algorithm(self) -> DigestAlgorithm {
        match self {
            Self::Sha256 => DigestAlgorithm::Sha256,
            Self::Sha384 => DigestAlgorithm::Sha384,
            Self::Sha512 => DigestAlgorithm::Sha512,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CspHashSourceValue<'a> {
    algorithm: CspHashAlgorithm,
    digest: &'a str,
}

fn hash_source_value(value: &str) -> Option<CspHashSourceValue<'_>> {
    let (algorithm, digest) = value.split_once('-')?;
    if digest.is_empty() {
        return None;
    }
    Some(CspHashSourceValue {
        algorithm: algorithm.parse().ok()?,
        digest,
    })
}

fn csp_hash_source_value(source: &str) -> Option<CspHashSourceValue<'_>> {
    let source = source.strip_prefix('\'')?.strip_suffix('\'')?;
    hash_source_value(source)
}

fn script_integrity_matches_hash_source(integrity: Option<&str>, sources: &[&str]) -> bool {
    let Some(integrity) = integrity else {
        return false;
    };
    // CSP requires a nonempty subset of its hash sources, including weaker
    // algorithms. SRI checks the strongest hashes against the response later.
    let mut hashes = integrity_metadata_hashes(integrity).peekable();
    hashes.peek().is_some()
        && hashes.all(|hash| {
            sources
                .iter()
                .filter_map(|source| csp_hash_source_value(source))
                .any(|source| {
                    source.algorithm.digest_algorithm() == hash.algorithm
                        && source.digest == hash.digest
                })
        })
}

fn source_expression_matches(
    source: &str,
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> bool {
    if csp_keyword_eq(source, "self") {
        return self_source_matches(protected_url, request_url);
    }
    if source == "*" {
        return matches!(request_url.scheme(), "http" | "https" | "ws" | "wss");
    }
    if let Some(scheme) = source.strip_suffix(':')
        && !scheme.contains('/')
        && !scheme.is_empty()
    {
        return csp_scheme_match(scheme, request_url.scheme()) != CspSchemeMatch::NotMatching;
    }
    source_url_matches(source, protected_url, request_url, redirect_status)
}

fn source_url_matches(
    source: &str,
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> bool {
    let Some(source) = parse_host_source(source) else {
        return false;
    };
    let source_scheme = source.scheme.unwrap_or_else(|| protected_url.scheme());
    let scheme_match = csp_scheme_match(source_scheme, request_url.scheme());
    if scheme_match == CspSchemeMatch::NotMatching {
        return false;
    }
    let port_match = host_source_port_match(source_scheme, source.port, request_url);
    if !csp_scheme_and_port_match(scheme_match, port_match) {
        return false;
    }
    let Some(request_host) = request_url.host_str() else {
        return false;
    };
    if !csp_host_part_matches(source.host, request_host) {
        return false;
    }
    // Keep the existing wildcard-port IPv4 restriction separate from grammar
    // validation; sources with ordinary ports already accept literal IPv4.
    if source.port == Some("*")
        && source.host != "127.0.0.1"
        && source.host.parse::<std::net::Ipv4Addr>().is_ok()
    {
        return false;
    }
    redirect_status == ContentSecurityPolicyRedirectStatus::FollowedRedirect
        || source
            .path
            .is_none_or(|path| csp_path_part_matches(path, request_url.path()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CspSchemeMatch {
    NotMatching,
    Exact,
    WebSocketToHttp,
    Upgrade,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CspPortMatch {
    NotMatching,
    Exact,
    Wildcard,
    Upgrade,
}

fn self_source_matches(protected_url: &Url, request_url: &Url) -> bool {
    if protected_url.host_str() != request_url.host_str() {
        return false;
    }
    let scheme_match = csp_scheme_match(protected_url.scheme(), request_url.scheme());
    let port_match = csp_port_match(
        protected_url.scheme(),
        protected_url.port_or_known_default(),
        false,
        request_url,
    );
    if scheme_match == CspSchemeMatch::Exact && port_match == CspPortMatch::Exact {
        return true;
    }
    let source_port_is_default = protected_url
        .port_or_known_default()
        .is_some_and(|port| is_default_port_for_scheme(port, protected_url.scheme()));
    let request_port_is_default = request_url
        .port_or_known_default()
        .is_some_and(|port| is_default_port_for_scheme(port, request_url.scheme()));
    let ports_match_or_defaults =
        port_match == CspPortMatch::Exact || (source_port_is_default && request_port_is_default);
    ports_match_or_defaults
        && (request_url.scheme().eq_ignore_ascii_case("https")
            || request_url.scheme().eq_ignore_ascii_case("wss")
            || protected_url.scheme().eq_ignore_ascii_case("http"))
}

fn csp_scheme_match(source_scheme: &str, request_scheme: &str) -> CspSchemeMatch {
    if source_scheme.eq_ignore_ascii_case(request_scheme) {
        return CspSchemeMatch::Exact;
    }
    // CSP permits WebSocket sources to match HTTP URLs at the same security
    // level. Keep this distinct from exact matching used by 'self' and from
    // secure upgrades, which also participate in port-upgrade checks.
    // https://w3c.github.io/webappsec-csp/#match-schemes
    if (source_scheme.eq_ignore_ascii_case("ws") && request_scheme.eq_ignore_ascii_case("http"))
        || (source_scheme.eq_ignore_ascii_case("wss")
            && request_scheme.eq_ignore_ascii_case("https"))
    {
        return CspSchemeMatch::WebSocketToHttp;
    }
    if (source_scheme.eq_ignore_ascii_case("http") && request_scheme.eq_ignore_ascii_case("https"))
        || (source_scheme.eq_ignore_ascii_case("ws")
            && (request_scheme.eq_ignore_ascii_case("wss")
                || request_scheme.eq_ignore_ascii_case("https")))
    {
        return CspSchemeMatch::Upgrade;
    }
    CspSchemeMatch::NotMatching
}

fn csp_port_match(
    source_scheme: &str,
    source_port: Option<u16>,
    source_port_wildcard: bool,
    request_url: &Url,
) -> CspPortMatch {
    if source_port_wildcard {
        return CspPortMatch::Wildcard;
    }
    let request_port = request_url.port_or_known_default();
    if csp_scheme_match(source_scheme, request_url.scheme()) == CspSchemeMatch::Upgrade
        && matches!(source_port, Some(80 | 443))
        && request_port == Some(443)
    {
        return CspPortMatch::Upgrade;
    }
    if source_port.is_some() && source_port == request_port {
        return CspPortMatch::Exact;
    }
    CspPortMatch::NotMatching
}

fn is_default_port_for_scheme(port: u16, scheme: &str) -> bool {
    matches!(
        (scheme.to_ascii_lowercase().as_str(), port),
        ("http", 80) | ("ws", 80) | ("https", 443) | ("wss", 443)
    )
}

fn csp_scheme_and_port_match(scheme_match: CspSchemeMatch, port_match: CspPortMatch) -> bool {
    if port_match == CspPortMatch::NotMatching {
        return false;
    }
    let requires_upgrade =
        scheme_match == CspSchemeMatch::Upgrade || port_match == CspPortMatch::Upgrade;
    if !requires_upgrade {
        return true;
    }
    let scheme_can_upgrade = scheme_match == CspSchemeMatch::Upgrade;
    let port_can_upgrade =
        port_match == CspPortMatch::Upgrade || port_match == CspPortMatch::Wildcard;
    scheme_can_upgrade && port_can_upgrade
}

struct CspHostSource<'a> {
    scheme: Option<&'a str>,
    host: &'a str,
    port: Option<&'a str>,
    path: Option<&'a str>,
}

fn parse_host_source(source: &str) -> Option<CspHostSource<'_>> {
    // CSP host-source is not a URL: credentials and scheme-relative URLs are
    // invalid, and URL parsing must not normalize hosts or dot segments.
    // https://w3c.github.io/webappsec-csp/#source-lists
    let (scheme, rest) = if let Some((scheme, rest)) = source.split_once("://")
        && !scheme.contains('/')
    {
        let mut bytes = scheme.bytes();
        if !bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            return None;
        }
        (Some(scheme), rest)
    } else {
        (None, source)
    };
    let (authority, path) = split_authority_and_path(rest);
    let (host, port) = if let Some((host, port)) = authority.split_once(':') {
        if port != "*" && (port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit())) {
            return None;
        }
        (host, Some(port))
    } else {
        (authority, None)
    };
    if host != "*" {
        let labels = host.strip_prefix("*.").unwrap_or(host);
        let labels = labels.strip_suffix('.').unwrap_or(labels);
        if !labels.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        }) {
            return None;
        }
    }
    if path.is_some_and(|path| !csp_path_part_is_valid(path)) {
        return None;
    }
    Some(CspHostSource {
        scheme,
        host,
        port,
        path,
    })
}

fn split_authority_and_path(source_rest: &str) -> (&str, Option<&str>) {
    source_rest
        .find('/')
        .map(|path_start| (&source_rest[..path_start], Some(&source_rest[path_start..])))
        .unwrap_or((source_rest, None))
}

fn csp_host_part_matches(source_host: &str, request_host: &str) -> bool {
    if source_host == "*" {
        return !request_host.is_empty();
    }
    if let Some(suffix) = source_host.strip_prefix('*') {
        return request_host.len() >= suffix.len()
            && request_host.as_bytes()[request_host.len() - suffix.len()..]
                .eq_ignore_ascii_case(suffix.as_bytes());
    }
    source_host.eq_ignore_ascii_case(request_host)
}

fn csp_path_part_is_valid(path: &str) -> bool {
    // RFC 3986 path-absolute, excluding raw ';' and ',' as required by CSP.
    if !path.starts_with('/') || path.starts_with("//") {
        return false;
    }
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            if !bytes.next().is_some_and(|byte| byte.is_ascii_hexdigit())
                || !bytes.next().is_some_and(|byte| byte.is_ascii_hexdigit())
            {
                return false;
            }
        } else if !byte.is_ascii_alphanumeric()
            && !matches!(
                byte,
                b'/' | b'-'
                    | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b'='
                    | b':'
                    | b'@'
            )
        {
            return false;
        }
    }
    true
}

fn csp_path_part_matches(source_path: &str, request_path: &str) -> bool {
    if source_path.is_empty() || (source_path == "/" && request_path.is_empty()) {
        return true;
    }
    // Split before decoding so an encoded slash cannot create a path boundary.
    // Compare bytes: distinct invalid UTF-8 sequences must not become equal.
    // https://w3c.github.io/webappsec-csp/#match-paths
    let mut source_segments = source_path.split('/').peekable();
    let mut request_segments = request_path.split('/');
    while let Some(source_segment) = source_segments.next() {
        let Some(request_segment) = request_segments.next() else {
            return false;
        };
        if source_segment.is_empty() && source_segments.peek().is_none() {
            return true;
        }
        if !percent_decode_str(source_segment).eq(percent_decode_str(request_segment)) {
            return false;
        }
    }
    request_segments.next().is_none()
}

fn host_source_port_match(
    source_scheme: &str,
    source_port: Option<&str>,
    request_url: &Url,
) -> CspPortMatch {
    match source_port {
        Some("*") => CspPortMatch::Wildcard,
        Some(port) => port
            .parse::<u16>()
            .ok()
            .map(|source_port| csp_port_match(source_scheme, Some(source_port), false, request_url))
            .unwrap_or(CspPortMatch::NotMatching),
        None => csp_port_match(
            source_scheme,
            default_port_for_scheme(source_scheme),
            false,
            request_url,
        ),
    }
}

fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

fn source_has_path(source: &str) -> bool {
    let after_scheme = source
        .split_once("://")
        .map(|(_, rest)| rest)
        .or_else(|| source.strip_prefix("//"))
        .unwrap_or(source);
    after_scheme.contains('/')
}

fn csp_keyword_eq(source: &str, keyword: &str) -> bool {
    source.len() == keyword.len() + 2
        && source.starts_with('\'')
        && source.ends_with('\'')
        && source[1..source.len() - 1].eq_ignore_ascii_case(keyword)
}

impl ContentSecurityPolicyResourceKind {
    fn effective_directive(self) -> &'static str {
        match self {
            Self::DocumentBase => BASE_URI,
            Self::DocumentConnect => CONNECT_SRC,
            Self::DocumentFrame => FRAME_SRC,
            Self::DocumentFont => FONT_SRC,
            Self::DocumentImage => IMG_SRC,
            Self::DocumentManifest => MANIFEST_SRC,
            Self::DocumentMedia => MEDIA_SRC,
            Self::DocumentFont => "font-src",
            Self::DocumentScriptElement | Self::WorkerDynamicModuleImport | Self::WorkerScript => SCRIPT_SRC_ELEM,
            Self::DocumentStyleElement => STYLE_SRC_ELEM,
            Self::WorkerConstructor | Self::WorkerStaticModuleImport => WORKER_SRC,
            Self::WorkerConnect => CONNECT_SRC,
        }
    }

    fn directive_fallbacks(self) -> &'static [&'static str] {
        match self {
            Self::DocumentBase => &[BASE_URI],
            Self::DocumentConnect => &[CONNECT_SRC, DEFAULT_SRC],
            Self::DocumentFrame => &[FRAME_SRC, CHILD_SRC, DEFAULT_SRC],
            Self::DocumentFont => &[FONT_SRC, DEFAULT_SRC],
            Self::DocumentImage => &[IMG_SRC, DEFAULT_SRC],
            Self::DocumentManifest => &[MANIFEST_SRC, DEFAULT_SRC],
            Self::DocumentMedia => &[MEDIA_SRC, DEFAULT_SRC],
            Self::DocumentFont => &["font-src", DEFAULT_SRC],
            Self::DocumentScriptElement => &[SCRIPT_SRC_ELEM, SCRIPT_SRC, DEFAULT_SRC],
            Self::DocumentStyleElement => &[STYLE_SRC_ELEM, STYLE_SRC, DEFAULT_SRC],
            Self::WorkerConstructor => &[WORKER_SRC, CHILD_SRC, SCRIPT_SRC, DEFAULT_SRC],
            Self::WorkerConnect => &[CONNECT_SRC, DEFAULT_SRC],
            Self::WorkerDynamicModuleImport => &[SCRIPT_SRC_ELEM, SCRIPT_SRC, DEFAULT_SRC],
            Self::WorkerScript => &[SCRIPT_SRC_ELEM, SCRIPT_SRC, DEFAULT_SRC],
            Self::WorkerStaticModuleImport => &[WORKER_SRC, CHILD_SRC, SCRIPT_SRC, DEFAULT_SRC],
        }
    }
}

impl ContentSecurityPolicyNonUrlKind {
    fn effective_directive(self) -> &'static str {
        match self {
            Self::DocumentInlineEventHandler => SCRIPT_SRC_ATTR,
            Self::DocumentInlineNavigation | Self::DocumentInlineScript => SCRIPT_SRC_ELEM,
            Self::DocumentInlineStyleAttribute => STYLE_SRC_ATTR,
            Self::DocumentInlineStyleElement => STYLE_SRC_ELEM,
            Self::Eval | Self::TrustedTypesEval => SCRIPT_SRC,
            Self::WasmEval => SCRIPT_SRC,
        }
    }

    fn directive_fallbacks(self) -> &'static [&'static str] {
        match self {
            Self::DocumentInlineEventHandler => &[SCRIPT_SRC_ATTR, SCRIPT_SRC, DEFAULT_SRC],
            Self::DocumentInlineNavigation | Self::DocumentInlineScript => {
                &[SCRIPT_SRC_ELEM, SCRIPT_SRC, DEFAULT_SRC]
            }
            Self::DocumentInlineStyleAttribute => &[STYLE_SRC_ATTR, STYLE_SRC, DEFAULT_SRC],
            Self::DocumentInlineStyleElement => &[STYLE_SRC_ELEM, STYLE_SRC, DEFAULT_SRC],
            Self::Eval | Self::TrustedTypesEval => &[SCRIPT_SRC, DEFAULT_SRC],
            Self::WasmEval => &[SCRIPT_SRC, DEFAULT_SRC],
        }
    }

    fn blocked_uri(self) -> &'static str {
        match self {
            Self::DocumentInlineEventHandler
            | Self::DocumentInlineNavigation
            | Self::DocumentInlineScript
            | Self::DocumentInlineStyleAttribute
            | Self::DocumentInlineStyleElement => "inline",
            Self::Eval | Self::TrustedTypesEval => "eval",
            Self::WasmEval => "wasm-eval",
        }
    }

    fn source_list_allows(self, source_list: &[&str], source: Option<&str>) -> bool {
        match self {
            Self::DocumentInlineEventHandler
            | Self::DocumentInlineNavigation
            | Self::DocumentInlineStyleAttribute => {
                inline_event_handler_source_list_allows(source_list, source.unwrap_or_default())
            }
            Self::DocumentInlineScript => source_list
                .iter()
                .any(|source| csp_keyword_eq(source, "unsafe-inline")),
            Self::DocumentInlineStyleElement => source_list
                .iter()
                .any(|source| csp_keyword_eq(source, "unsafe-inline")),
            Self::Eval => source_list
                .iter()
                .any(|source| csp_keyword_eq(source, "unsafe-eval")),
            Self::TrustedTypesEval => source_list.iter().any(|source| {
                csp_keyword_eq(source, "unsafe-eval")
                    || csp_keyword_eq(source, "trusted-types-eval")
            }),
            Self::WasmEval => source_list.iter().any(|source| {
                csp_keyword_eq(source, "unsafe-eval") || csp_keyword_eq(source, "wasm-unsafe-eval")
            }),
        }
    }
}

fn inline_event_handler_source_list_allows(source_list: &[&str], source: &str) -> bool {
    let has_nonce_or_hash = source_list.iter().any(|source| {
        csp_nonce_source_value(source).is_some() || csp_hash_source_value(source).is_some()
    });
    if !has_nonce_or_hash
        && source_list
            .iter()
            .any(|source| csp_keyword_eq(source, "unsafe-inline"))
    {
        return true;
    }
    if !source_list
        .iter()
        .any(|source| csp_keyword_eq(source, "unsafe-hashes"))
    {
        return false;
    }
    source_list
        .iter()
        .filter_map(|source| csp_hash_source_value(source))
        .any(|hash_source| inline_source_matches_hash(source, hash_source))
}

fn inline_source_matches_hash(source: &str, hash_source: CspHashSourceValue<'_>) -> bool {
    let digest = hash_source
        .algorithm
        .digest_algorithm()
        .digest_bytes(source.as_bytes());
    let actual = BASE64_STANDARD.encode(digest);
    // Inline checks normalize base64url symbols, preserving padding and all
    // other bytes. External script integrity matches remain literal.
    actual
        .bytes()
        .eq(hash_source.digest.bytes().map(|byte| match byte {
            b'-' => b'+',
            b'_' => b'/',
            _ => byte,
        }))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

fn trusted_types_violation_sample(sample: &str) -> String {
    sample.chars().take(40).collect()
}

pub fn content_security_policy_trusted_types_policy_violation_with_disposition_and_reporting_endpoints(
    policy: &ContentSecurityPolicy,
    protected_url: &Url,
    policy_name: &str,
    is_duplicate: bool,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    if policy_allows_trusted_type_policy_name(policy, policy_name, is_duplicate) {
        return None;
    }
    let document_uri = csp_url_for_report(protected_url);
    Some(ContentSecurityPolicyUrlViolation {
        effective_directive: TRUSTED_TYPES,
        blocked_uri: "trusted-types-policy".to_owned(),
        source_file: document_uri.clone(),
        document_uri,
        original_policy: policy.as_str().to_owned(),
        disposition,
        report_uri_endpoints: content_security_policy_report_uri_endpoints(policy, protected_url),
        report_to_endpoints: content_security_policy_report_to_endpoints(
            policy,
            reporting_endpoints,
        ),
        sample: trusted_types_violation_sample(policy_name),
        line_number: 0,
        column_number: 0,
    })
}

#[path = "embedded_enforcement.rs"]
mod embedded_enforcement;
pub use embedded_enforcement::RequiredCspResponse;
