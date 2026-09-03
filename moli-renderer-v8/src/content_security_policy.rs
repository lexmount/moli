use std::collections::BTreeMap;

use crate::context_bootstrap::{initialize_event_object, mark_event_trusted};
use crate::network::ResourceRequestClient;
use crate::util::v8_string;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_crypto::DigestAlgorithm;
use moli_fetch::{
    Request, RequestCredentialsMode, RequestMode, RequestRedirectMode, RequestResourceType,
};
use moli_webapi_declare::WebApiObject;
use serde_json::json;
use url::Url;

const CONNECT_SRC: &str = "connect-src";
const CHILD_SRC: &str = "child-src";
const DEFAULT_SRC: &str = "default-src";
const FRAME_ANCESTORS: &str = "frame-ancestors";
const FRAME_SRC: &str = "frame-src";
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContentSecurityPolicyResourceKind {
    DocumentConnect,
    DocumentFrame,
    DocumentImage,
    DocumentManifest,
    DocumentMedia,
    DocumentScriptElement,
    DocumentStyleElement,
    WorkerConstructor,
    WorkerConnect,
    WorkerDynamicModuleImport,
    WorkerScript,
    WorkerStaticModuleImport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContentSecurityPolicyNonUrlKind {
    DocumentInlineEventHandler,
    DocumentInlineNavigation,
    DocumentInlineScript,
    DocumentInlineStyleAttribute,
    DocumentInlineStyleElement,
    Eval,
    TrustedTypesEval,
    WasmEval,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContentSecurityPolicyRedirectStatus {
    NoRedirect,
    FollowedRedirect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContentSecurityPolicyDisposition {
    Enforce,
    Report,
}

impl ContentSecurityPolicyDisposition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Report => "report",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TrustedTypesForScriptRequirements {
    enforced: bool,
    report_only: bool,
}

impl TrustedTypesForScriptRequirements {
    pub(crate) const fn new(enforced: bool, report_only: bool) -> Self {
        Self {
            enforced,
            report_only,
        }
    }

    pub(crate) const fn enforced_only(enforced: bool) -> Self {
        Self::new(enforced, false)
    }

    pub(crate) const fn requires_conversion(self) -> bool {
        self.enforced || self.report_only
    }

    pub(crate) const fn is_enforced(self) -> bool {
        self.enforced
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentSecurityPolicyScriptElementRequest<'a> {
    pub(crate) nonce: Option<&'a str>,
    pub(crate) integrity: Option<&'a str>,
    pub(crate) parser_inserted: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentSecurityPolicyStyleElementRequest<'a> {
    pub(crate) nonce: Option<&'a str>,
}

impl Default for ContentSecurityPolicyScriptElementRequest<'_> {
    fn default() -> Self {
        Self {
            nonce: None,
            integrity: None,
            parser_inserted: true,
        }
    }
}

impl<'a> ContentSecurityPolicyScriptElementRequest<'a> {
    pub(crate) fn parser_inserted_with_nonce(nonce: Option<&'a str>) -> Self {
        Self {
            nonce,
            integrity: None,
            parser_inserted: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContentSecurityPolicyUrlViolation {
    pub(crate) effective_directive: &'static str,
    pub(crate) blocked_uri: String,
    pub(crate) document_uri: String,
    pub(crate) original_policy: String,
    pub(crate) disposition: ContentSecurityPolicyDisposition,
    pub(crate) report_uri_endpoints: Vec<String>,
    pub(crate) report_to_endpoints: Vec<String>,
    pub(crate) sample: String,
    pub(crate) source_file: String,
    pub(crate) line_number: i32,
    pub(crate) column_number: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentSecurityPolicyReportingEndpoints {
    endpoints: BTreeMap<String, String>,
}

impl ContentSecurityPolicyReportingEndpoints {
    pub(crate) fn endpoint_for_group(&self, group: &str) -> Option<&str> {
        self.endpoints.get(group).map(String::as_str)
    }

    fn insert(&mut self, group: impl Into<String>, endpoint: impl Into<String>) {
        self.endpoints.insert(group.into(), endpoint.into());
    }
}

pub(crate) struct ContentSecurityPolicyViolationEventFields<'a> {
    pub(crate) document_uri: &'a str,
    pub(crate) referrer: &'a str,
    pub(crate) blocked_uri: &'a str,
    pub(crate) effective_directive: &'a str,
    pub(crate) violated_directive: &'a str,
    pub(crate) original_policy: &'a str,
    pub(crate) disposition: ContentSecurityPolicyDisposition,
    pub(crate) source_file: &'a str,
    pub(crate) sample: &'a str,
    pub(crate) line_number: i32,
    pub(crate) column_number: i32,
    pub(crate) status_code: i32,
}

#[derive(WebApiObject)]
#[webapi(
    interface = "SecurityPolicyViolationEvent",
    data_properties,
    enumerable
)]
struct ContentSecurityPolicyViolationEventDeclaration<'scope> {
    #[webapi(data_property = "documentURI")]
    document_uri: v8::Local<'scope, v8::String>,
    referrer: v8::Local<'scope, v8::String>,
    #[webapi(data_property = "blockedURI")]
    blocked_uri: v8::Local<'scope, v8::String>,
    effective_directive: v8::Local<'scope, v8::String>,
    violated_directive: v8::Local<'scope, v8::String>,
    original_policy: v8::Local<'scope, v8::String>,
    disposition: v8::Local<'scope, v8::String>,
    source_file: v8::Local<'scope, v8::String>,
    sample: v8::Local<'scope, v8::String>,
    line_number: i32,
    column_number: i32,
    status_code: i32,
}

impl<'a> ContentSecurityPolicyViolationEventFields<'a> {
    pub(crate) fn from_url_violation(violation: &'a ContentSecurityPolicyUrlViolation) -> Self {
        Self {
            document_uri: violation.document_uri.as_str(),
            referrer: "",
            blocked_uri: violation.blocked_uri.as_str(),
            effective_directive: violation.effective_directive,
            violated_directive: violation.effective_directive,
            original_policy: violation.original_policy.as_str(),
            disposition: violation.disposition,
            source_file: violation.source_file.as_str(),
            sample: violation.sample.as_str(),
            line_number: 0,
            column_number: 0,
            status_code: 0,
        }
    }
}

pub(crate) fn create_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let declaration = security_policy_violation_event_declaration(scope, fields)?;
    let event = declaration.bind(scope).ok()?;
    initialize_event_object(scope, event, "securitypolicyviolation", true, false);
    mark_event_trusted(scope, event);
    Some(event)
}

pub(crate) fn initialize_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> bool {
    security_policy_violation_event_declaration(scope, fields)
        .and_then(|declaration| declaration.initialize(scope, event).ok())
        .is_some()
}

fn security_policy_violation_event_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> Option<ContentSecurityPolicyViolationEventDeclaration<'s>> {
    let document_uri = v8_string(scope, fields.document_uri)?;
    let referrer = v8_string(scope, fields.referrer)?;
    let blocked_uri = v8_string(scope, fields.blocked_uri)?;
    let effective_directive = v8_string(scope, fields.effective_directive)?;
    let violated_directive = v8_string(scope, fields.violated_directive)?;
    let original_policy = v8_string(scope, fields.original_policy)?;
    let disposition = v8_string(scope, fields.disposition.as_str())?;
    let source_file = v8_string(scope, fields.source_file)?;
    let sample = v8_string(scope, fields.sample)?;
    Some(ContentSecurityPolicyViolationEventDeclaration {
        document_uri,
        referrer,
        blocked_uri,
        effective_directive,
        violated_directive,
        original_policy,
        disposition,
        source_file,
        sample,
        line_number: fields.line_number,
        column_number: fields.column_number,
        status_code: fields.status_code,
    })
}

pub(crate) fn content_security_policy_headers(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-security-policy"))
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) fn content_security_policy_report_only_headers(
    headers: &[(String, String)],
) -> Vec<String> {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-security-policy-report-only"))
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) fn content_security_policy_requires_trusted_types_for_script(
    policies: &[String],
) -> bool {
    policies
        .iter()
        .any(|policy| policy_requires_trusted_types_for_script(policy))
}

pub(crate) fn content_security_policy_allows_trusted_types_eval(policies: &[String]) -> bool {
    // Like Blink's ContentSecurityPolicy::AllowTrustedTypesEval, this only
    // activates the document-global Trusted Types relaxation. It is not the
    // final CSP decision: CSP3 EnsureCSPDoesNotBlockStringCompilation still
    // evaluates every policy through the TrustedTypesEval non-URL gate.
    policies
        .iter()
        .any(|policy| policy_requires_trusted_types_for_script(policy))
        && policies
            .iter()
            .any(|policy| policy_allows_trusted_types_eval(policy))
}

pub(crate) fn content_security_policy_sandboxes_document_domain(policies: &[String]) -> bool {
    policies
        .iter()
        .any(|policy| policy_sandboxes_document_domain(policy))
}

pub(crate) fn content_security_policy_forces_opaque_origin(policies: &[String]) -> bool {
    policies
        .iter()
        .any(|policy| policy_forces_opaque_origin(policy))
}

pub(crate) fn content_security_policy_sandbox_allows_scripts(policies: &[String]) -> bool {
    policies
        .iter()
        .all(|policy| policy_sandbox_allows_scripts(policy).unwrap_or(true))
}

pub(crate) fn content_security_policy_sandbox_allows_modals(policies: &[String]) -> bool {
    policies
        .iter()
        .all(|policy| policy_sandbox_allows_modals(policy).unwrap_or(true))
}

pub(crate) fn content_security_policy_sandbox_allows_popups_to_escape(policies: &[String]) -> bool {
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

pub(crate) fn content_security_policy_report_uri_endpoints(
    policy: &str,
    protected_url: &Url,
) -> Vec<String> {
    let directives = parsed_directives(policy);
    if directive_source_list(&directives, "report-to").is_some() {
        return Vec::new();
    }
    directives
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("report-uri"))
        .flat_map(|(_, values)| values.iter())
        .filter_map(|endpoint| protected_url.join(endpoint).ok())
        .filter(|endpoint| matches!(endpoint.scheme(), "http" | "https"))
        .map(|endpoint| endpoint.to_string())
        .collect()
}

pub(crate) fn content_security_policy_reporting_endpoints_from_headers(
    headers: &[(String, String)],
    protected_url: &Url,
) -> ContentSecurityPolicyReportingEndpoints {
    let mut endpoints = ContentSecurityPolicyReportingEndpoints::default();
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("reporting-endpoints"))
    {
        parse_reporting_endpoints_header(value, protected_url, &mut endpoints);
    }
    endpoints
}

fn parse_reporting_endpoints_header(
    value: &str,
    protected_url: &Url,
    endpoints: &mut ContentSecurityPolicyReportingEndpoints,
) {
    for member in split_quoted_header_list(value) {
        let Some((raw_group, raw_endpoint)) = member.split_once('=') else {
            continue;
        };
        let group = raw_group.trim();
        if group.is_empty() {
            continue;
        }
        let Some(endpoint) = parse_quoted_header_value(raw_endpoint.trim()) else {
            continue;
        };
        let Ok(endpoint) = protected_url.join(&endpoint) else {
            continue;
        };
        if !matches!(endpoint.scheme(), "http" | "https") {
            continue;
        }
        endpoints.insert(group, endpoint.to_string());
    }
}

fn split_quoted_header_list(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quote = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            ',' if !in_quote => {
                let part = value[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let part = value[start..].trim();
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

fn parse_quoted_header_value(value: &str) -> Option<String> {
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            output.push(chars.next()?);
        } else {
            output.push(ch);
        }
    }
    Some(output)
}

pub(crate) fn content_security_policy_report_to_endpoints(
    policy: &str,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Vec<String> {
    let directives = parsed_directives(policy);
    let Some(sources) = directive_source_list(&directives, "report-to") else {
        return Vec::new();
    };
    let Some(group) = sources.first() else {
        return Vec::new();
    };
    reporting_endpoints
        .endpoint_for_group(group)
        .map(|endpoint| vec![endpoint.to_owned()])
        .unwrap_or_default()
}

pub(crate) fn content_security_policy_violation_report_body(
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> String {
    json!({
        "csp-report": {
            "document-uri": fields.document_uri,
            "referrer": fields.referrer,
            "violated-directive": fields.violated_directive,
            "effective-directive": fields.effective_directive,
            "original-policy": fields.original_policy,
            "disposition": fields.disposition.as_str(),
            "blocked-uri": fields.blocked_uri,
            "source-file": fields.source_file,
            "status-code": fields.status_code,
            "script-sample": fields.sample,
        }
    })
    .to_string()
}

pub(crate) fn content_security_policy_reporting_api_report_body(
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> String {
    let mut body = serde_json::Map::new();
    body.insert("documentURL".to_owned(), json!(fields.document_uri));
    if !fields.referrer.is_empty() {
        body.insert("referrer".to_owned(), json!(fields.referrer));
    }
    if !fields.blocked_uri.is_empty() {
        body.insert("blockedURL".to_owned(), json!(fields.blocked_uri));
    }
    body.insert(
        "effectiveDirective".to_owned(),
        json!(fields.effective_directive),
    );
    body.insert("originalPolicy".to_owned(), json!(fields.original_policy));
    if !fields.source_file.is_empty() {
        body.insert("sourceFile".to_owned(), json!(fields.source_file));
    }
    if !fields.sample.is_empty() {
        body.insert("sample".to_owned(), json!(fields.sample));
    }
    body.insert("disposition".to_owned(), json!(fields.disposition.as_str()));
    body.insert("statusCode".to_owned(), json!(fields.status_code));
    if fields.line_number != 0 {
        body.insert("lineNumber".to_owned(), json!(fields.line_number));
    }
    if fields.column_number != 0 {
        body.insert("columnNumber".to_owned(), json!(fields.column_number));
    }
    json!([{
        "age": 0,
        "type": "csp-violation",
        "url": fields.document_uri,
        "body": body,
    }])
    .to_string()
}

pub(crate) fn content_security_policy_report_requests(
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
    report_uri_endpoints: &[String],
    report_to_endpoints: &[String],
) -> Vec<Request> {
    let mut requests = Vec::new();
    append_content_security_policy_report_requests(
        &mut requests,
        &content_security_policy_violation_report_body(fields),
        "application/csp-report",
        report_uri_endpoints,
    );
    append_content_security_policy_report_requests(
        &mut requests,
        &content_security_policy_reporting_api_report_body(fields),
        "application/reports+json",
        report_to_endpoints,
    );
    requests
}

fn append_content_security_policy_report_requests(
    requests: &mut Vec<Request>,
    body: &str,
    content_type: &str,
    endpoints: &[String],
) {
    for endpoint in endpoints {
        let Ok(request) = Request::new_bytes(
            "POST",
            endpoint,
            Some(body.as_bytes().to_vec()),
            vec![("Content-Type".to_owned(), content_type.to_owned())],
        ) else {
            continue;
        };
        requests.push(
            request
                .with_resource_type(RequestResourceType::CspReport)
                .with_request_mode(RequestMode::NoCors)
                .with_credentials_mode(RequestCredentialsMode::SameOrigin)
                .with_redirect_mode(RequestRedirectMode::Error),
        );
    }
}

pub(crate) fn send_content_security_policy_reports(
    loader: &ResourceRequestClient,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
    report_uri_endpoints: &[String],
    report_to_endpoints: &[String],
) {
    for request in
        content_security_policy_report_requests(fields, report_uri_endpoints, report_to_endpoints)
    {
        send_content_security_policy_report_request(loader, request);
    }
}

fn send_content_security_policy_report_request(loader: &ResourceRequestClient, request: Request) {
    if let Err(error) = loader.fetch_text_callback(request, |result| {
        if let Err(error) = result {
            tracing::debug!(message = error.to_string(), "CSP report delivery failed");
        }
    }) {
        tracing::debug!(
            message = error.to_string(),
            "CSP report request submission failed"
        );
    }
}

pub(crate) fn ensure_content_security_policy_allows_url(
    policies: &[String],
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

pub(crate) fn ensure_content_security_policy_allows_url_with_redirect_status(
    policies: &[String],
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
    policies: &[String],
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

pub(crate) fn content_security_policy_allows_url_with_redirect_status(
    policies: &[String],
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
    policies: &[String],
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
    policies: &[String],
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

pub(crate) fn content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
    policies: &[String],
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

pub(crate) fn content_security_policy_frame_ancestors_violation_with_disposition_and_reporting_endpoints(
    policies: &[String],
    protected_url: &Url,
    ancestor_origins: &[Option<Url>],
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    policies.iter().find_map(|policy| {
        let directives = parsed_directives(policy);
        let source_list =
            normalized_source_list(directive_source_list(&directives, FRAME_ANCESTORS)?.to_vec());
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
            original_policy: policy.clone(),
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
pub(crate) fn content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
    policies: &[String],
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
pub(crate) fn content_security_policy_style_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
    policies: &[String],
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
pub(crate) fn content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
    policies: &[String],
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
    policies: &[String],
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
            original_policy: policy.clone(),
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

fn csp_url_for_report(url: &Url) -> String {
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return url.scheme().to_owned();
    }
    let mut stripped = url.clone();
    let _ = stripped.set_username("");
    let _ = stripped.set_password(None);
    stripped.set_fragment(None);
    stripped.to_string()
}

/// Strip a script source URL before exposing it through a CSP violation report.
///
/// This implements the URL-shaping portion of CSP's "Strip URL for use in
/// reports" algorithm. Script source locations may expose HTTP(S)/WS(S) paths,
/// but never credentials, queries, or fragments. Other valid schemes are
/// reduced to the scheme name.
pub(crate) fn content_security_policy_source_file_for_report(source_file: &str) -> String {
    let Ok(mut source_url) = Url::parse(source_file) else {
        return String::new();
    };
    if !matches!(source_url.scheme(), "http" | "https" | "ws" | "wss") {
        return source_url.scheme().to_owned();
    }
    let _ = source_url.set_username("");
    let _ = source_url.set_password(None);
    source_url.set_query(None);
    source_url.set_fragment(None);
    source_url.to_string()
}

pub(crate) fn content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints(
    policies: &[String],
    protected_url: &Url,
    sink: &str,
    sample: &str,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    policies.iter().find_map(|policy| {
        if !policy_requires_trusted_types_for_script(policy) {
            return None;
        }
        let document_uri = protected_url.to_string();
        Some(ContentSecurityPolicyUrlViolation {
            effective_directive: REQUIRE_TRUSTED_TYPES_FOR,
            blocked_uri: "trusted-types-sink".to_owned(),
            source_file: document_uri.clone(),
            document_uri,
            original_policy: policy.clone(),
            disposition,
            report_uri_endpoints: content_security_policy_report_uri_endpoints(
                policy,
                protected_url,
            ),
            report_to_endpoints: content_security_policy_report_to_endpoints(
                policy,
                reporting_endpoints,
            ),
            sample: trusted_types_sink_violation_sample(sink, sample),
            line_number: 0,
            column_number: 0,
        })
    })
}

pub(crate) fn content_security_policy_trusted_types_policy_violation_with_disposition_and_reporting_endpoints(
    policies: &[String],
    protected_url: &Url,
    policy_name: &str,
    is_duplicate: bool,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    policies.iter().find_map(|policy| {
        if policy_allows_trusted_type_policy_name(policy, policy_name, is_duplicate) {
            return None;
        }
        let document_uri = protected_url.to_string();
        Some(ContentSecurityPolicyUrlViolation {
            effective_directive: TRUSTED_TYPES,
            blocked_uri: "trusted-types-policy".to_owned(),
            source_file: document_uri.clone(),
            document_uri,
            original_policy: policy.clone(),
            disposition,
            report_uri_endpoints: content_security_policy_report_uri_endpoints(
                policy,
                protected_url,
            ),
            report_to_endpoints: content_security_policy_report_to_endpoints(
                policy,
                reporting_endpoints,
            ),
            sample: trusted_types_violation_sample(policy_name),
            line_number: 0,
            column_number: 0,
        })
    })
}

pub(crate) fn content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
    policy: &str,
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

pub(crate) fn content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
    policy: &str,
    protected_url: &Url,
    source: &str,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    let kind = ContentSecurityPolicyNonUrlKind::DocumentInlineScript;
    let directives = parsed_directives(policy);
    let (effective_directive, source_list) =
        kind.directive_fallbacks().iter().find_map(|directive| {
            directive_source_list(&directives, directive)
                .map(|sources| (kind.effective_directive(), sources.to_vec()))
        })?;
    if inline_script_element_source_list_allows(source_list.clone(), source, request) {
        return None;
    }
    let document_uri = protected_url.to_string();
    Some(ContentSecurityPolicyUrlViolation {
        effective_directive,
        blocked_uri: kind.blocked_uri().to_owned(),
        source_file: document_uri.clone(),
        document_uri,
        original_policy: policy.to_owned(),
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

pub(crate) fn content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
    policy: &str,
    protected_url: &Url,
    source: &str,
    request: ContentSecurityPolicyStyleElementRequest<'_>,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    let kind = ContentSecurityPolicyNonUrlKind::DocumentInlineStyleElement;
    let directives = parsed_directives(policy);
    let (effective_directive, source_list) =
        kind.directive_fallbacks().iter().find_map(|directive| {
            directive_source_list(&directives, directive)
                .map(|sources| (kind.effective_directive(), sources.to_vec()))
        })?;
    if inline_style_element_source_list_allows(source_list.clone(), source, request) {
        return None;
    }
    let document_uri = protected_url.to_string();
    Some(ContentSecurityPolicyUrlViolation {
        effective_directive,
        blocked_uri: kind.blocked_uri().to_owned(),
        source_file: document_uri.clone(),
        document_uri,
        original_policy: policy.to_owned(),
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

pub(crate) fn content_security_policy_non_url_violation_with_source(
    policy: &str,
    protected_url: &Url,
    kind: ContentSecurityPolicyNonUrlKind,
    source: Option<&str>,
    disposition: ContentSecurityPolicyDisposition,
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Option<ContentSecurityPolicyUrlViolation> {
    let directives = parsed_directives(policy);
    let (effective_directive, source_list) =
        kind.directive_fallbacks().iter().find_map(|directive| {
            directive_source_list(&directives, directive)
                .map(|sources| (kind.effective_directive(), sources.to_vec()))
        })?;
    if kind.source_list_allows(&source_list, source) {
        return None;
    }
    let document_uri = protected_url.to_string();
    Some(ContentSecurityPolicyUrlViolation {
        effective_directive,
        blocked_uri: kind.blocked_uri().to_owned(),
        source_file: document_uri.clone(),
        document_uri,
        original_policy: policy.to_owned(),
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

fn policy_requires_trusted_types_for_script(policy: &str) -> bool {
    let directives = parsed_directives(policy);
    directive_source_list(&directives, REQUIRE_TRUSTED_TYPES_FOR).is_some_and(|sources| {
        sources
            .iter()
            .any(|source| csp_keyword_eq(source, "script"))
    })
}

fn policy_allows_trusted_types_eval(policy: &str) -> bool {
    let directives = parsed_directives(policy);
    [SCRIPT_SRC, DEFAULT_SRC]
        .into_iter()
        .find_map(|directive| directive_source_list(&directives, directive))
        .is_some_and(|sources| {
            sources
                .iter()
                .any(|source| csp_keyword_eq(source.trim(), "trusted-types-eval"))
        })
}

fn policy_allows_trusted_type_policy_name(
    policy: &str,
    policy_name: &str,
    is_duplicate: bool,
) -> bool {
    let directives = parsed_directives(policy);
    let Some(sources) = directive_source_list(&directives, TRUSTED_TYPES) else {
        return true;
    };
    let name_is_allowed = sources.iter().any(|source| {
        let source = source.trim();
        source == "*"
            || (!source.is_empty()
                && !csp_keyword_eq(source, "none")
                && !csp_keyword_eq(source, "allow-duplicates")
                && source == policy_name)
    });
    let duplicate_is_allowed = !is_duplicate
        || sources
            .iter()
            .any(|source| csp_keyword_eq(source.trim(), "allow-duplicates"));
    name_is_allowed && duplicate_is_allowed
}

fn policy_sandboxes_document_domain(policy: &str) -> bool {
    let directives = parsed_directives(policy);
    directive_source_list(&directives, SANDBOX).is_some()
}

fn policy_forces_opaque_origin(policy: &str) -> bool {
    let directives = parsed_directives(policy);
    directive_source_list(&directives, SANDBOX)
        .is_some_and(|sources| !sandbox_sources_allow_same_origin(sources))
}

fn policy_sandbox_allows_scripts(policy: &str) -> Option<bool> {
    let directives = parsed_directives(policy);
    directive_source_list(&directives, SANDBOX).map(sandbox_sources_allow_scripts)
}

fn policy_sandbox_allows_modals(policy: &str) -> Option<bool> {
    let directives = parsed_directives(policy);
    directive_source_list(&directives, SANDBOX).map(sandbox_sources_allow_modals)
}

fn policy_sandbox_allows_popups_to_escape(policy: &str) -> Option<bool> {
    let directives = parsed_directives(policy);
    directive_source_list(&directives, SANDBOX).map(sandbox_sources_allow_popups_to_escape)
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

fn trusted_types_violation_sample(sample: &str) -> String {
    sample.chars().take(40).collect()
}

fn effective_source_list_with_directive(
    policy: &str,
    kind: ContentSecurityPolicyResourceKind,
) -> Option<(&'static str, Vec<&str>)> {
    let directives = parsed_directives(policy);
    kind.directive_fallbacks().iter().find_map(|directive| {
        directive_source_list(&directives, directive)
            .map(|sources| (kind.effective_directive(), sources.to_vec()))
    })
}

fn parsed_directives(policy: &str) -> Vec<(&str, Vec<&str>)> {
    policy
        .split(';')
        .filter_map(|directive| {
            let mut parts = directive.split_ascii_whitespace();
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            Some((name, parts.collect()))
        })
        .collect()
}

fn directive_source_list<'policy, 'directives>(
    directives: &'directives [(&'policy str, Vec<&'policy str>)],
    name: &str,
) -> Option<&'directives [&'policy str]> {
    directives
        .iter()
        .find(|(directive_name, _)| directive_name.eq_ignore_ascii_case(name))
        .map(|(_, sources)| sources.as_slice())
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
        .any(|source| csp_keyword_eq(source.trim(), "report-sample"))
    {
        return String::new();
    }
    source.chars().take(40).collect()
}

fn normalized_source_list(sources: Vec<&str>) -> Vec<&str> {
    sources
        .into_iter()
        .map(str::trim)
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
    let source = source.trim();
    let Some(value) = source
        .strip_prefix("'nonce-")
        .and_then(|value| value.strip_suffix('\''))
    else {
        return false;
    };
    value == nonce
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
    source
        .trim()
        .strip_prefix("'nonce-")
        .and_then(|value| value.strip_suffix('\''))
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
    let source = source.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    hash_source_value(source)
}

fn script_integrity_matches_hash_source(integrity: Option<&str>, sources: &[&str]) -> bool {
    let Some(integrity) = integrity else {
        return false;
    };
    integrity.split_whitespace().any(|metadata| {
        let metadata = metadata.split_once('?').map_or(metadata, |(hash, _)| hash);
        let Some(metadata) = hash_source_value(metadata) else {
            return false;
        };
        sources
            .iter()
            .filter_map(|source| csp_hash_source_value(source))
            .any(|source| source == metadata)
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
    if host_source_contains_query_or_fragment(source) {
        return false;
    }
    if let Some(matches) =
        wildcard_host_source_matches(source, protected_url, request_url, redirect_status)
    {
        return matches;
    }
    let Ok(source_url) = parse_source_url(source, protected_url) else {
        return false;
    };
    let scheme_match = csp_scheme_match(source_url.scheme(), request_url.scheme());
    if scheme_match == CspSchemeMatch::NotMatching {
        return false;
    }
    if source_url.host_str() != request_url.host_str() {
        return false;
    }
    let port_match = csp_port_match(
        source_url.scheme(),
        source_url.port_or_known_default(),
        false,
        request_url,
    );
    if !csp_scheme_and_port_match(scheme_match, port_match) {
        return false;
    }
    if redirect_status == ContentSecurityPolicyRedirectStatus::FollowedRedirect
        || !source_has_path(source)
    {
        return true;
    }
    let source_path = source_url.path();
    let request_path = request_url.path();
    if source_path.ends_with('/') {
        request_path.starts_with(source_path)
    } else {
        request_path == source_path
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CspSchemeMatch {
    NotMatching,
    Exact,
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
    if (source_scheme.eq_ignore_ascii_case("http") && request_scheme.eq_ignore_ascii_case("https"))
        || (source_scheme.eq_ignore_ascii_case("ws") && request_scheme.eq_ignore_ascii_case("wss"))
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

fn wildcard_host_source_matches(
    source: &str,
    protected_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Option<bool> {
    let (source_scheme, rest) = if let Some((scheme, rest)) = source.split_once("://") {
        (Some(scheme), rest)
    } else if let Some(rest) = source.strip_prefix("//") {
        (Some(protected_url.scheme()), rest)
    } else {
        (None, source)
    };
    let (authority, source_path) = split_authority_and_path(rest);
    let (source_host, source_port) = split_source_authority(authority)?;
    if source_host != "*" && !source_host.starts_with("*.") {
        return None;
    }
    let source_scheme = source_scheme.unwrap_or_else(|| protected_url.scheme());
    let scheme_match = csp_scheme_match(source_scheme, request_url.scheme());
    if scheme_match == CspSchemeMatch::NotMatching {
        return Some(false);
    }
    let port_match = wildcard_source_port_match(source_scheme, source_port, request_url);
    if !csp_scheme_and_port_match(scheme_match, port_match) {
        return Some(false);
    }
    let Some(request_host) = request_url.host_str() else {
        return Some(false);
    };
    if !wildcard_source_host_matches(source_host, request_host) {
        return Some(false);
    }
    if let Some(source_path) = source_path
        && redirect_status == ContentSecurityPolicyRedirectStatus::NoRedirect
    {
        let request_path = request_url.path();
        return Some(if source_path.ends_with('/') {
            request_path.starts_with(source_path)
        } else {
            request_path == source_path
        });
    }
    Some(true)
}

fn split_authority_and_path(source_rest: &str) -> (&str, Option<&str>) {
    source_rest
        .find('/')
        .map(|path_start| (&source_rest[..path_start], Some(&source_rest[path_start..])))
        .unwrap_or((source_rest, None))
}

fn split_source_authority(authority: &str) -> Option<(&str, Option<&str>)> {
    if authority.is_empty() {
        return None;
    }
    if let Some((host, port)) = authority.rsplit_once(':')
        && !host.is_empty()
        && (port == "*" || port.parse::<u16>().is_ok())
    {
        return Some((host, Some(port)));
    }
    Some((authority, None))
}

fn wildcard_source_host_matches(source_host: &str, request_host: &str) -> bool {
    let source_host = source_host.trim_end_matches('.').to_ascii_lowercase();
    let request_host = request_host.trim_end_matches('.').to_ascii_lowercase();
    if source_host == "*" {
        return !request_host.is_empty();
    }
    let Some(source_suffix) = source_host.strip_prefix("*.") else {
        return false;
    };
    request_host.len() > source_suffix.len()
        && request_host.ends_with(source_suffix)
        && request_host.as_bytes()[request_host.len() - source_suffix.len() - 1] == b'.'
}

fn wildcard_source_port_match(
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
        _ => None,
    }
}

fn parse_source_url(source: &str, protected_url: &Url) -> Result<Url, url::ParseError> {
    if source.contains("://") {
        Url::parse(source)
    } else if source.starts_with("//") {
        Url::parse(&format!("{}:{source}", protected_url.scheme()))
    } else {
        Url::parse(&format!("{}://{}", protected_url.scheme(), source))
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

fn host_source_contains_query_or_fragment(source: &str) -> bool {
    let after_scheme = source
        .split_once("://")
        .map(|(_, rest)| rest)
        .or_else(|| source.strip_prefix("//"))
        .unwrap_or(source);
    after_scheme.contains('?') || after_scheme.contains('#')
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
            Self::DocumentConnect => CONNECT_SRC,
            Self::DocumentFrame => FRAME_SRC,
            Self::DocumentImage => IMG_SRC,
            Self::DocumentManifest => MANIFEST_SRC,
            Self::DocumentMedia => MEDIA_SRC,
            Self::DocumentScriptElement | Self::WorkerDynamicModuleImport => SCRIPT_SRC_ELEM,
            Self::DocumentStyleElement => STYLE_SRC_ELEM,
            Self::WorkerConstructor | Self::WorkerStaticModuleImport => WORKER_SRC,
            Self::WorkerConnect => CONNECT_SRC,
            Self::WorkerScript => SCRIPT_SRC,
        }
    }

    fn directive_fallbacks(self) -> &'static [&'static str] {
        match self {
            Self::DocumentConnect => &[CONNECT_SRC, DEFAULT_SRC],
            Self::DocumentFrame => &[FRAME_SRC, CHILD_SRC, DEFAULT_SRC],
            Self::DocumentImage => &[IMG_SRC, DEFAULT_SRC],
            Self::DocumentManifest => &[MANIFEST_SRC, DEFAULT_SRC],
            Self::DocumentMedia => &[MEDIA_SRC, DEFAULT_SRC],
            Self::DocumentScriptElement => &[SCRIPT_SRC_ELEM, SCRIPT_SRC, DEFAULT_SRC],
            Self::DocumentStyleElement => &[STYLE_SRC_ELEM, STYLE_SRC, DEFAULT_SRC],
            Self::WorkerConstructor => &[WORKER_SRC, CHILD_SRC, SCRIPT_SRC, DEFAULT_SRC],
            Self::WorkerConnect => &[CONNECT_SRC, DEFAULT_SRC],
            Self::WorkerDynamicModuleImport => &[SCRIPT_SRC_ELEM, SCRIPT_SRC, DEFAULT_SRC],
            Self::WorkerScript => &[SCRIPT_SRC, DEFAULT_SRC],
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
                .any(|source| csp_keyword_eq(source.trim(), "unsafe-inline")),
            Self::DocumentInlineStyleElement => source_list
                .iter()
                .any(|source| csp_keyword_eq(source.trim(), "unsafe-inline")),
            Self::Eval => source_list
                .iter()
                .any(|source| csp_keyword_eq(source.trim(), "unsafe-eval")),
            Self::TrustedTypesEval => source_list.iter().any(|source| {
                let source = source.trim();
                csp_keyword_eq(source, "unsafe-eval")
                    || csp_keyword_eq(source, "trusted-types-eval")
            }),
            Self::WasmEval => source_list.iter().any(|source| {
                let source = source.trim();
                csp_keyword_eq(source, "unsafe-eval") || csp_keyword_eq(source, "wasm-unsafe-eval")
            }),
        }
    }
}

fn inline_event_handler_source_list_allows(source_list: &[&str], source: &str) -> bool {
    let source_list = source_list
        .iter()
        .map(|source| source.trim())
        .collect::<Vec<_>>();
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
    actual == hash_source.digest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protected_url() -> Url {
        Url::parse("https://app.test/page.html").unwrap()
    }

    fn request_url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    fn allowed(policy: &str, kind: ContentSecurityPolicyResourceKind, request: &str) -> bool {
        content_security_policy_allows_url(
            &[policy.to_owned()],
            &protected_url(),
            &request_url(request),
            kind,
        )
    }

    fn frame_ancestors_violation(
        policies: &[&str],
        protected_url: &str,
        ancestor_origins: &[Option<&str>],
    ) -> Option<ContentSecurityPolicyUrlViolation> {
        content_security_policy_frame_ancestors_violation_with_disposition_and_reporting_endpoints(
            &policies
                .iter()
                .map(|policy| (*policy).to_owned())
                .collect::<Vec<_>>(),
            &request_url(protected_url),
            &ancestor_origins
                .iter()
                .map(|ancestor| ancestor.map(request_url))
                .collect::<Vec<_>>(),
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
        )
    }

    #[test]
    fn content_security_policy_headers_collect_enforce_headers_only() {
        let headers = vec![
            (
                "Content-Security-Policy-Report-Only".to_owned(),
                "worker-src 'none'".to_owned(),
            ),
            (
                "content-security-policy".to_owned(),
                " worker-src 'self' ".to_owned(),
            ),
            ("Content-Security-Policy".to_owned(), String::new()),
        ];

        assert_eq!(
            content_security_policy_headers(&headers),
            vec!["worker-src 'self'".to_owned()]
        );
        assert_eq!(
            content_security_policy_report_only_headers(&headers),
            vec!["worker-src 'none'".to_owned()]
        );
    }

    #[test]
    fn frame_ancestors_requires_every_ancestor_to_match() {
        let protected_url = "https://child.test/frame.html";
        let ancestors = [Some("https://child.test"), Some("https://top.test")];

        let violation =
            frame_ancestors_violation(&["frame-ancestors 'self'"], protected_url, &ancestors)
                .expect("a cross-origin top ancestor must violate 'self'");
        assert_eq!(violation.effective_directive, "frame-ancestors");
        assert_eq!(violation.blocked_uri, protected_url);

        assert!(
            frame_ancestors_violation(&["frame-ancestors *"], protected_url, &ancestors).is_none()
        );
        assert!(
            frame_ancestors_violation(
                &["frame-ancestors https://child.test https://top.test"],
                protected_url,
                &ancestors,
            )
            .is_none()
        );
        assert!(
            frame_ancestors_violation(
                &["frame-ancestors https://child.test"],
                protected_url,
                &ancestors,
            )
            .is_some()
        );
    }

    #[test]
    fn frame_ancestors_has_no_fallback_and_allows_top_level_documents() {
        assert!(
            frame_ancestors_violation(
                &["default-src 'none'"],
                "https://child.test/frame.html",
                &[Some("https://top.test")],
            )
            .is_none()
        );
        assert!(
            frame_ancestors_violation(
                &["frame-ancestors 'none'"],
                "https://child.test/frame.html",
                &[],
            )
            .is_none()
        );
    }

    #[test]
    fn every_frame_ancestors_policy_must_allow_embedding() {
        let violation = frame_ancestors_violation(
            &["frame-ancestors *", "frame-ancestors 'none'"],
            "https://child.test/frame.html",
            &[Some("https://top.test")],
        )
        .expect("one blocking policy must block the response");
        assert_eq!(violation.original_policy, "frame-ancestors 'none'");
    }

    #[test]
    fn frame_ancestors_matches_origins_and_rejects_opaque_ancestors() {
        assert!(
            frame_ancestors_violation(
                &["frame-ancestors https://top.test/path"],
                "https://child.test/frame.html",
                &[Some("https://top.test")],
            )
            .is_some(),
            "a host source containing a path must not match an ancestor origin"
        );
        assert!(
            frame_ancestors_violation(
                &["frame-ancestors *"],
                "https://child.test/frame.html",
                &[None],
            )
            .is_some(),
            "an opaque ancestor origin must not match a source expression"
        );
    }

    #[test]
    fn worker_src_none_blocks_worker_constructor() {
        assert!(!allowed(
            "worker-src 'none'; script-src 'self'",
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            "https://app.test/worker.js"
        ));
    }

    #[test]
    fn worker_constructor_uses_script_src_and_default_src_fallbacks() {
        assert!(!allowed(
            "script-src 'none'",
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            "https://app.test/worker.js"
        ));
        assert!(!allowed(
            "default-src 'none'",
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            "https://app.test/worker.js"
        ));
        assert!(allowed(
            "default-src 'self'",
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            "https://app.test/worker.js"
        ));
    }

    #[test]
    fn worker_constructor_uses_child_src_before_script_src_fallback() {
        assert!(!allowed(
            "child-src 'none'; script-src 'self'",
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            "https://app.test/worker.js"
        ));
        assert!(allowed(
            "child-src https://workers.test; script-src 'none'",
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            "https://workers.test/worker.js"
        ));
    }

    #[test]
    fn document_script_element_uses_script_src_elem_fallbacks() {
        assert!(!allowed(
            "script-src-elem 'none'; script-src 'self'",
            ContentSecurityPolicyResourceKind::DocumentScriptElement,
            "https://app.test/app.js"
        ));
        assert!(allowed(
            "script-src 'self'; default-src 'none'",
            ContentSecurityPolicyResourceKind::DocumentScriptElement,
            "https://app.test/app.js"
        ));
        assert!(!allowed(
            "default-src 'none'",
            ContentSecurityPolicyResourceKind::DocumentScriptElement,
            "https://app.test/app.js"
        ));
    }

    #[test]
    fn worker_src_takes_precedence_for_worker_constructors() {
        assert!(allowed(
            "default-src 'none'; script-src 'none'; worker-src 'self'",
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            "https://app.test/worker.js"
        ));
        assert!(!allowed(
            "default-src *; script-src *; worker-src 'none'",
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            "https://app.test/worker.js"
        ));
    }

    #[test]
    fn url_violation_reports_effective_directive_and_original_policy() {
        let violation = content_security_policy_url_violation_with_redirect_status(
            &[
                "connect-src https://api.test".to_owned(),
                "default-src 'none'".to_owned(),
            ],
            &protected_url(),
            &request_url("https://blocked.test/data.json"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .expect("blocked worker connect should produce violation");

        assert_eq!(violation.effective_directive, "connect-src");
        assert_eq!(violation.blocked_uri, "https://blocked.test/data.json");
        assert_eq!(violation.document_uri, "https://app.test/page.html");
        assert_eq!(violation.original_policy, "connect-src https://api.test");
        assert_eq!(
            violation.disposition,
            ContentSecurityPolicyDisposition::Enforce
        );
    }

    #[test]
    fn url_violation_strips_credentials_and_fragments_from_report_urls() {
        let protected_url =
            request_url("https://user:secret@app.test/page.html?document=1#current-section");
        let blocked_url =
            request_url("https://other:secret@blocked.test/image.png?asset=1#pixel-fragment");
        let violation = content_security_policy_url_violation_with_redirect_status(
            &["img-src 'none'".to_owned()],
            &protected_url,
            &blocked_url,
            ContentSecurityPolicyResourceKind::DocumentImage,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .expect("blocked image should produce violation");

        assert_eq!(
            violation.document_uri,
            "https://app.test/page.html?document=1"
        );
        assert_eq!(
            violation.blocked_uri,
            "https://blocked.test/image.png?asset=1"
        );
        assert_eq!(
            violation.source_file,
            "https://app.test/page.html?document=1"
        );
    }

    #[test]
    fn url_violation_redacts_data_and_blob_urls_to_their_schemes() {
        for (policy, url, expected) in [
            ("media-src 'self'", "data:video/mp4;base64,AAAA", "data"),
            (
                "media-src https://example.com",
                "blob:https://app.test/id",
                "blob",
            ),
        ] {
            let violation = content_security_policy_url_violation_with_redirect_status(
                &[policy.to_owned()],
                &protected_url(),
                &request_url(url),
                ContentSecurityPolicyResourceKind::DocumentMedia,
                ContentSecurityPolicyRedirectStatus::NoRedirect,
            )
            .expect("blocked media URL should produce violation");

            assert_eq!(violation.effective_directive, "media-src");
            assert_eq!(violation.blocked_uri, expected);
        }
    }

    #[test]
    fn redirected_url_violation_reports_original_blocked_uri() {
        let original_url = request_url("https://app.test/redirect");
        let final_url = request_url("https://blocked.test/data.json");
        let violation = content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
            &["connect-src 'self'".to_owned()],
            &protected_url(),
            &final_url,
            &original_url,
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::FollowedRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
        )
        .expect("redirected blocked worker connect should produce violation");

        assert_eq!(violation.effective_directive, "connect-src");
        assert_eq!(violation.blocked_uri, "https://app.test/redirect");
    }

    #[test]
    fn self_source_matches_same_origin_only() {
        assert!(allowed(
            "script-src 'self'",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://app.test/worker-import.js"
        ));
        assert!(!allowed(
            "script-src 'self'",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://cdn.test/worker-import.js"
        ));
    }

    #[test]
    fn worker_module_imports_use_their_request_specific_directive_fallbacks() {
        let cross_origin = "https://cdn.test/worker-import.js";

        assert!(!allowed(
            "worker-src 'self'; script-src *",
            ContentSecurityPolicyResourceKind::WorkerStaticModuleImport,
            cross_origin,
        ));
        assert!(allowed(
            "worker-src *; script-src 'self'",
            ContentSecurityPolicyResourceKind::WorkerStaticModuleImport,
            cross_origin,
        ));
        assert!(!allowed(
            "script-src-elem 'self'; script-src *",
            ContentSecurityPolicyResourceKind::WorkerDynamicModuleImport,
            cross_origin,
        ));
        assert!(allowed(
            "worker-src 'self'; script-src *",
            ContentSecurityPolicyResourceKind::WorkerDynamicModuleImport,
            cross_origin,
        ));

        let violation = content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
            &["script-src 'self'".to_owned()],
            &protected_url(),
            &request_url(cross_origin),
            ContentSecurityPolicyResourceKind::WorkerDynamicModuleImport,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
        )
        .expect("script-src fallback should block the cross-origin dynamic import");
        assert_eq!(violation.effective_directive, "script-src-elem");
    }

    #[test]
    fn self_source_uses_csp_secure_upgrade_rules() {
        let protected = Url::parse("http://app.test/page.html").unwrap();
        assert!(content_security_policy_allows_url(
            &["connect-src 'self'".to_owned()],
            &protected,
            &request_url("https://app.test/api"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
        ));
        assert!(content_security_policy_allows_url(
            &["connect-src 'self'".to_owned()],
            &protected,
            &request_url("ws://app.test/socket"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
        ));
        assert!(content_security_policy_allows_url(
            &["connect-src 'self'".to_owned()],
            &protected,
            &request_url("wss://app.test/socket"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
        ));

        let protected = Url::parse("https://app.test/page.html").unwrap();
        assert!(content_security_policy_allows_url(
            &["connect-src 'self'".to_owned()],
            &protected,
            &request_url("wss://app.test/socket"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
        ));
        assert!(!content_security_policy_allows_url(
            &["connect-src 'self'".to_owned()],
            &protected,
            &request_url("ws://app.test/socket"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
        ));
        assert!(!content_security_policy_allows_url(
            &["connect-src 'self'".to_owned()],
            &Url::parse("http://app.test/page.html").unwrap(),
            &request_url("ftp://app.test/resource"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
        ));
    }

    #[test]
    fn connect_src_and_default_src_control_worker_connect_requests() {
        assert!(!allowed(
            "connect-src 'none'",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "https://app.test/api"
        ));
        assert!(!allowed(
            "default-src 'none'",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "https://app.test/api"
        ));
        assert!(allowed(
            "connect-src 'self'; default-src 'none'",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "https://app.test/api"
        ));
    }

    #[test]
    fn wildcard_source_does_not_match_local_schemes() {
        assert!(allowed(
            "script-src *",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://app.test/worker-import.js"
        ));
        assert!(!allowed(
            "script-src *",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "data:text/javascript,onconnect=()=>{}"
        ));
    }

    #[test]
    fn scheme_sources_use_csp_secure_upgrade_rules() {
        assert!(allowed(
            "connect-src http:",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "https://cdn.test/api"
        ));
        assert!(allowed(
            "connect-src ws:",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "wss://cdn.test/socket"
        ));
        assert!(!allowed(
            "connect-src https:",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "wss://cdn.test/socket"
        ));
        assert!(!allowed(
            "connect-src http:",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "ws://cdn.test/socket"
        ));
    }

    #[test]
    fn source_path_matching_uses_encoded_paths() {
        assert!(allowed(
            "script-src https://app.test/trusted/",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://app.test/trusted/worker.js"
        ));
        assert!(!allowed(
            "script-src https://app.test/trusted/",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://app.test/trusted%2Fevil.js"
        ));
    }

    #[test]
    fn host_sources_support_scheme_relative_urls() {
        assert!(allowed(
            "script-src //cdn.test",
            ContentSecurityPolicyResourceKind::DocumentScriptElement,
            "https://cdn.test/app.js"
        ));
        assert!(!allowed(
            "script-src //cdn.test",
            ContentSecurityPolicyResourceKind::DocumentScriptElement,
            "http://cdn.test/app.js"
        ));
    }

    #[test]
    fn host_sources_reject_query_and_fragment_components() {
        assert!(!allowed(
            "script-src https://cdn.test?ignored",
            ContentSecurityPolicyResourceKind::DocumentScriptElement,
            "https://cdn.test/app.js"
        ));
        assert!(!allowed(
            "script-src https://cdn.test/app.js#fragment",
            ContentSecurityPolicyResourceKind::DocumentScriptElement,
            "https://cdn.test/app.js"
        ));
    }

    #[test]
    fn redirected_source_url_matching_ignores_path_but_not_host() {
        assert!(content_security_policy_allows_url_with_redirect_status(
            &["script-src https://cdn.test/trusted/".to_owned()],
            &protected_url(),
            &request_url("https://cdn.test/evil.js"),
            ContentSecurityPolicyResourceKind::WorkerScript,
            ContentSecurityPolicyRedirectStatus::FollowedRedirect,
        ));
        assert!(!content_security_policy_allows_url_with_redirect_status(
            &["script-src https://cdn.test/trusted/".to_owned()],
            &protected_url(),
            &request_url("https://evil.test/evil.js"),
            ContentSecurityPolicyResourceKind::WorkerScript,
            ContentSecurityPolicyRedirectStatus::FollowedRedirect,
        ));
    }

    #[test]
    fn wildcard_host_source_matches_subdomains_only() {
        assert!(allowed(
            "script-src *.cdn.example.com",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://assets.cdn.example.com/worker-import.js"
        ));
        assert!(allowed(
            "script-src https://*.cdn.example.com",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://assets.cdn.example.com/worker-import.js"
        ));
        assert!(!allowed(
            "script-src *.cdn.example.com",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://cdn.example.com/worker-import.js"
        ));
        assert!(!allowed(
            "script-src *.cdn.example.com",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://evilcdn.example.com/worker-import.js"
        ));
    }

    #[test]
    fn wildcard_host_source_honors_port_and_path() {
        assert!(allowed(
            "script-src https://*.cdn.example.com:*",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://assets.cdn.example.com:8443/worker-import.js"
        ));
        assert!(!allowed(
            "script-src https://*.cdn.example.com",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://assets.cdn.example.com:8443/worker-import.js"
        ));
        assert!(allowed(
            "script-src https://*.cdn.example.com/trusted/",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://assets.cdn.example.com/trusted/worker-import.js"
        ));
        assert!(!allowed(
            "script-src https://*.cdn.example.com/trusted/",
            ContentSecurityPolicyResourceKind::WorkerScript,
            "https://assets.cdn.example.com/trusted%2Fevil.js"
        ));
    }

    #[test]
    fn host_sources_use_csp_secure_upgrade_rules() {
        assert!(allowed(
            "connect-src http://api.example.com",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "https://api.example.com/socket"
        ));
        assert!(allowed(
            "connect-src ws://api.example.com",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "wss://api.example.com/socket"
        ));
        assert!(!allowed(
            "connect-src https://api.example.com",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "wss://api.example.com/socket"
        ));
        assert!(!allowed(
            "connect-src http://api.example.com",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "https://api.example.com:80/socket"
        ));
    }

    #[test]
    fn wildcard_host_sources_use_csp_secure_upgrade_rules() {
        assert!(allowed(
            "connect-src ws://*.cdn.example.com",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "wss://assets.cdn.example.com/socket"
        ));
        assert!(!allowed(
            "connect-src https://*.cdn.example.com",
            ContentSecurityPolicyResourceKind::WorkerConnect,
            "wss://assets.cdn.example.com/socket"
        ));
    }

    #[test]
    fn all_policies_are_enforced() {
        let result = ensure_content_security_policy_allows_url(
            &[
                "script-src 'self'".to_owned(),
                "script-src 'none'".to_owned(),
            ],
            &protected_url(),
            &request_url("https://app.test/worker-import.js"),
            ContentSecurityPolicyResourceKind::WorkerScript,
            || "blocked".to_owned(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn script_element_nonce_source_allows_external_script_url() {
        let violation =
            content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
                &["script-src 'nonce-abc'".to_owned()],
                &protected_url(),
                &request_url("https://cdn.test/script.js"),
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
                ContentSecurityPolicyScriptElementRequest::parser_inserted_with_nonce(Some("abc")),
            );

        assert!(
            violation.is_none(),
            "matching script nonce should allow the script element request"
        );
    }

    #[test]
    fn style_element_request_uses_style_src_elem_fallbacks_and_nonce() {
        let protected_url = Url::parse("https://example.test/page").unwrap();
        let request_url = Url::parse("https://cdn.test/app.css").unwrap();
        let check = |policy: &str, nonce: Option<&str>| {
            content_security_policy_style_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
                &[policy.to_owned()],
                &protected_url,
                &request_url,
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
                ContentSecurityPolicyStyleElementRequest { nonce },
            )
        };

        assert!(check("style-src https://cdn.test", None).is_none());
        assert!(check("default-src https://cdn.test", None).is_none());
        let violation = check("style-src-elem 'none'; style-src https://cdn.test", None)
            .expect("style-src-elem must take precedence over style-src");
        assert_eq!(violation.effective_directive, "style-src-elem");
        assert!(
            check(
                "style-src-elem 'nonce-allowed'; style-src 'none'",
                Some("allowed")
            )
            .is_none(),
            "a matching link nonce must authorize the external style request"
        );
        assert!(
            check(
                "style-src-elem 'nonce-allowed'; style-src https://cdn.test",
                Some("blocked")
            )
            .is_some(),
            "a non-matching nonce must not bypass the operative style-src-elem directive"
        );
    }

    #[test]
    fn script_element_strict_dynamic_allows_non_parser_inserted_script() {
        let violation =
            content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
                &["script-src 'strict-dynamic' 'nonce-abc'".to_owned()],
                &protected_url(),
                &request_url("https://cdn.test/script.js"),
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
                ContentSecurityPolicyScriptElementRequest {
                    nonce: None,
                    integrity: None,
                    parser_inserted: false,
                },
            );

        assert!(
            violation.is_none(),
            "strict-dynamic should allow script elements inserted by a running trusted script"
        );
    }

    #[test]
    fn script_element_strict_dynamic_blocks_parser_inserted_host_source_without_nonce() {
        let violation =
            content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
                &["script-src 'strict-dynamic' 'nonce-abc' https://cdn.test".to_owned()],
                &protected_url(),
                &request_url("https://cdn.test/script.js"),
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
                ContentSecurityPolicyScriptElementRequest {
                    nonce: None,
                    integrity: None,
                    parser_inserted: true,
                },
            )
            .expect("parser-inserted script without nonce should be blocked");

        assert_eq!(violation.effective_directive, "script-src-elem");
    }

    #[test]
    fn script_element_integrity_hash_source_allows_external_script_url() {
        let violation =
            content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
                &["script-src 'ShA256-testdigest'".to_owned()],
                &protected_url(),
                &request_url("https://cdn.test/script.js"),
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
                ContentSecurityPolicyScriptElementRequest {
                    nonce: None,
                    integrity: Some("sha256-testdigest"),
                    parser_inserted: true,
                },
            );

        assert!(
            violation.is_none(),
            "matching integrity metadata should satisfy a CSP hash source regardless of algorithm case"
        );
    }

    #[test]
    fn script_src_elem_directive_controls_strict_dynamic_script_element_requests() {
        let allowed =
            content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
                &["script-src-elem 'strict-dynamic' 'nonce-abc'; script-src 'nonce-abc'".to_owned()],
                &protected_url(),
                &request_url("https://cdn.test/script.js"),
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
                ContentSecurityPolicyScriptElementRequest {
                    nonce: None,
                    integrity: None,
                    parser_inserted: false,
                },
            );
        assert!(
            allowed.is_none(),
            "script-src-elem strict-dynamic should control script element requests"
        );

        let violation =
            content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
                &["script-src 'strict-dynamic' 'nonce-abc'; script-src-elem 'nonce-abc'".to_owned()],
                &protected_url(),
                &request_url("https://cdn.test/script.js"),
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
                ContentSecurityPolicyScriptElementRequest {
                    nonce: None,
                    integrity: None,
                    parser_inserted: false,
                },
            )
            .expect("script-src-elem without strict-dynamic should block this request");

        assert_eq!(violation.effective_directive, "script-src-elem");
    }

    #[test]
    fn report_uri_endpoints_resolve_against_protected_url() {
        let endpoints = content_security_policy_report_uri_endpoints(
            "connect-src 'none'; report-uri /csp-report https://reports.test/collect",
            &protected_url(),
        );

        assert_eq!(
            endpoints,
            vec![
                "https://app.test/csp-report".to_owned(),
                "https://reports.test/collect".to_owned()
            ]
        );
    }

    #[test]
    fn report_to_suppresses_legacy_report_uri_transport() {
        let endpoints = content_security_policy_report_uri_endpoints(
            "connect-src 'none'; report-uri /legacy; report-to default",
            &protected_url(),
        );

        assert!(endpoints.is_empty());
    }

    #[test]
    fn reporting_endpoints_header_resolves_groups_against_protected_url() {
        let endpoints = content_security_policy_reporting_endpoints_from_headers(
            &[
                (
                    "Reporting-Endpoints".to_owned(),
                    "default=\"/reports\", csp=\"https://reports.test/csp\"".to_owned(),
                ),
                (
                    "reporting-endpoints".to_owned(),
                    "default=\"/override\", ignored=ftp://reports.test/nope".to_owned(),
                ),
            ],
            &protected_url(),
        );

        assert_eq!(
            endpoints.endpoint_for_group("default"),
            Some("https://app.test/override")
        );
        assert_eq!(
            endpoints.endpoint_for_group("csp"),
            Some("https://reports.test/csp")
        );
        assert_eq!(endpoints.endpoint_for_group("ignored"), None);
    }

    #[test]
    fn report_to_uses_first_group_endpoint_and_suppresses_report_uri() {
        let reporting_endpoints = content_security_policy_reporting_endpoints_from_headers(
            &[(
                "Reporting-Endpoints".to_owned(),
                "primary=\"/reports/primary\", secondary=\"/reports/secondary\"".to_owned(),
            )],
            &protected_url(),
        );
        let violation =
            content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
                &[
                    "connect-src 'none'; report-uri /legacy; report-to primary secondary"
                        .to_owned(),
                ],
                &protected_url(),
                &request_url("https://api.test/data.json"),
                ContentSecurityPolicyResourceKind::WorkerConnect,
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
            .expect("blocked URL should produce violation");

        assert!(violation.report_uri_endpoints.is_empty());
        assert_eq!(
            violation.report_to_endpoints,
            vec!["https://app.test/reports/primary".to_owned()]
        );
    }

    #[test]
    fn violation_report_body_uses_csp_report_shape() {
        let violation = content_security_policy_url_violation_with_redirect_status(
            &["connect-src 'none'; report-uri /csp-report".to_owned()],
            &protected_url(),
            &request_url("https://api.test/data.json"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .expect("blocked URL should produce violation");
        let body = content_security_policy_violation_report_body(
            &ContentSecurityPolicyViolationEventFields::from_url_violation(&violation),
        );
        let value: serde_json::Value = serde_json::from_str(&body).expect("report body JSON");

        assert_eq!(
            value,
            serde_json::json!({
                "csp-report": {
                    "document-uri": "https://app.test/page.html",
                    "referrer": "",
                    "violated-directive": "connect-src",
                    "effective-directive": "connect-src",
                    "original-policy": "connect-src 'none'; report-uri /csp-report",
                    "disposition": "enforce",
                    "blocked-uri": "https://api.test/data.json",
                    "source-file": "https://app.test/page.html",
                    "status-code": 0,
                    "script-sample": "",
                }
            })
        );
        assert_eq!(
            violation.report_uri_endpoints,
            vec!["https://app.test/csp-report".to_owned()]
        );
    }

    #[test]
    fn violation_report_request_uses_csp_fetch_security_modes() {
        let violation = content_security_policy_url_violation_with_redirect_status(
            &["connect-src 'none'; report-uri /csp-report".to_owned()],
            &protected_url(),
            &request_url("https://api.test/data.json"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .expect("blocked URL should produce violation");
        let requests = content_security_policy_report_requests(
            &ContentSecurityPolicyViolationEventFields::from_url_violation(&violation),
            &violation.report_uri_endpoints,
            &violation.report_to_endpoints,
        );

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].request_mode, RequestMode::NoCors);
        assert_eq!(
            requests[0].credentials_mode,
            RequestCredentialsMode::SameOrigin
        );
        assert_eq!(requests[0].redirect_mode, RequestRedirectMode::Error);
    }

    #[test]
    fn reporting_api_report_body_uses_csp_violation_shape() {
        let violation = content_security_policy_url_violation_with_redirect_status(
            &["connect-src 'none'".to_owned()],
            &protected_url(),
            &request_url("https://api.test/data.json"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .expect("blocked URL should produce violation");
        let body = content_security_policy_reporting_api_report_body(
            &ContentSecurityPolicyViolationEventFields::from_url_violation(&violation),
        );
        let value: serde_json::Value = serde_json::from_str(&body).expect("report body JSON");

        assert_eq!(value[0]["type"], "csp-violation");
        assert_eq!(value[0]["url"], "https://app.test/page.html");
        assert_eq!(
            value[0]["body"]["documentURL"],
            "https://app.test/page.html"
        );
        assert_eq!(value[0]["body"]["blockedURL"], "https://api.test/data.json");
        assert_eq!(value[0]["body"]["effectiveDirective"], "connect-src");
        assert_eq!(value[0]["body"]["originalPolicy"], "connect-src 'none'");
        assert_eq!(value[0]["body"]["disposition"], "enforce");
    }

    #[test]
    fn non_url_inline_script_uses_script_src_elem_fallbacks() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        assert!(
            content_security_policy_non_url_violation_with_source(
                "script-src-elem 'unsafe-inline'; script-src 'none'",
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::DocumentInlineScript,
                None,
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
            .is_none()
        );

        let violation = content_security_policy_non_url_violation_with_source(
            "script-src 'self'; default-src 'unsafe-inline'",
            &protected_url(),
            ContentSecurityPolicyNonUrlKind::DocumentInlineScript,
            None,
            ContentSecurityPolicyDisposition::Report,
            &reporting_endpoints,
        )
        .expect("script-src should block inline script without unsafe-inline");
        assert_eq!(violation.effective_directive, "script-src-elem");
        assert_eq!(violation.blocked_uri, "inline");
        assert_eq!(
            violation.disposition,
            ContentSecurityPolicyDisposition::Report
        );
    }

    #[test]
    fn inline_script_element_accepts_matching_nonce_or_source_hash() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        let source = "globalThis.inlineScriptRan = true;";
        let matching_hash = "'sha256-BAF+pD4OL3OpwYDBVElncnexa2ZpNAjS3yn9BmiJEto='";
        let check = |policy: &str, nonce: Option<&str>| {
            content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                source,
                ContentSecurityPolicyScriptElementRequest {
                    nonce,
                    integrity: None,
                    parser_inserted: true,
                },
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
        };

        assert!(check("script-src 'nonce-abc'", Some("abc")).is_none());
        assert!(check("script-src 'nonce-abc'", Some("wrong")).is_some());
        assert!(check(&format!("script-src {matching_hash}"), None).is_none());
        assert!(
            content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
                &format!("script-src {matching_hash}"),
                &protected_url(),
                "globalThis.inlineScriptRan = false;",
                ContentSecurityPolicyScriptElementRequest::default(),
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
            .is_some()
        );
    }

    #[test]
    fn inline_script_element_unsafe_inline_obeys_nonce_and_strict_dynamic_precedence() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        let check = |policy: &str, parser_inserted: bool| {
            content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                "globalThis.inlineScriptRan = true;",
                ContentSecurityPolicyScriptElementRequest {
                    nonce: None,
                    integrity: None,
                    parser_inserted,
                },
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
        };

        assert!(check("script-src 'unsafe-inline'", true).is_none());
        assert!(check("script-src 'unsafe-inline' 'nonce-abc'", true).is_some());
        assert!(check("script-src 'unsafe-inline' 'strict-dynamic'", true).is_some());
        assert!(check("script-src 'unsafe-inline' 'strict-dynamic'", false).is_none());
    }

    #[test]
    fn inline_style_element_uses_style_src_elem_and_accepts_nonce_or_hash() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        let source = "p { color: green; }";
        let matching_hash = "'sha256-FSRZotz4y83Ib8ZaoVj9eXKaeWXVUawM8zAPfYeYySs='";
        let check = |policy: &str, nonce: Option<&str>| {
            content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                source,
                ContentSecurityPolicyStyleElementRequest { nonce },
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
        };

        assert!(check("style-src-elem 'nonce-abc'; style-src 'none'", Some("abc")).is_none());
        assert!(
            check(
                "style-src-elem 'nonce-abc'; style-src 'unsafe-inline'",
                None
            )
            .is_some()
        );
        assert!(check(&format!("style-src {matching_hash}"), None).is_none());
        assert!(check("default-src 'unsafe-inline'", None).is_none());
        assert!(check("style-src 'unsafe-inline' 'nonce-abc'", None).is_some());
    }

    #[test]
    fn inline_hash_source_algorithm_names_are_ascii_case_insensitive() {
        let source = "p { color: green; }";
        let cases = [
            ("SHA256", "FSRZotz4y83Ib8ZaoVj9eXKaeWXVUawM8zAPfYeYySs="),
            (
                "sHa384",
                "XObbzlg3meDKX3QpZZz8hg0V39bxZI/QQ3qFg6emWOdBZinTtjOls/GimTwBU+WQ",
            ),
            (
                "Sha512",
                "yYhoHoYeH6+9nQONNTy2KAd3tps87iT2d7E58PcEnU5YmJh2am3blPT0gLXu3qnVxDvF/qQKibav9lIK3pCz1A==",
            ),
        ];

        for (algorithm, digest) in cases {
            let expression = format!("'{algorithm}-{digest}'");
            let hash_source = csp_hash_source_value(&expression)
                .expect("supported hash algorithm should parse case-insensitively");
            assert!(inline_source_matches_hash(source, hash_source));
        }
        assert!(csp_hash_source_value("'SHA1-digest'").is_none());
    }

    #[test]
    fn inline_style_attribute_uses_style_src_attr_and_requires_unsafe_hashes() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        let source = "background: green";
        let matching_hash = "'sha256-S0VSqEOmzmyOifPfat2sJ7ELOgkldAEbaXlvi5iMqjc='";
        let check = |policy: &str| {
            content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::DocumentInlineStyleAttribute,
                source,
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
        };

        assert!(check("style-src-attr 'unsafe-inline'; style-src 'none'").is_none());
        assert!(check("style-src-attr 'none'; style-src 'unsafe-inline'").is_some());
        assert!(check(&format!("style-src 'unsafe-hashes' {matching_hash}")).is_none());
        assert!(check(&format!("style-src {matching_hash}")).is_some());
        assert!(check("default-src 'unsafe-inline'").is_none());
    }

    #[test]
    fn inline_style_violation_sample_requires_report_sample_and_is_clipped() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        let source = "0123456789012345678901234567890123456789extra";
        let check_element = |policy: &str| {
            content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                source,
                ContentSecurityPolicyStyleElementRequest::default(),
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
            .expect("inline style should be blocked")
        };
        let check_attribute = |policy: &str| {
            content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::DocumentInlineStyleAttribute,
                source,
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
            .expect("inline style attribute should be blocked")
        };

        assert_eq!(check_element("style-src 'none'").sample, "");
        assert_eq!(
            check_element("style-src 'none' 'report-sample'").sample,
            "0123456789012345678901234567890123456789"
        );
        assert_eq!(
            check_attribute("style-src-attr 'none' 'report-sample'").sample,
            "0123456789012345678901234567890123456789"
        );
    }

    #[test]
    fn inline_event_handler_uses_script_src_attr_fallbacks() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        let check = |policy: &str, source: &str| {
            content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
                source,
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
        };

        assert!(
            check(
                "script-src-attr 'unsafe-inline'; script-src 'none'",
                "run() "
            )
            .is_none()
        );
        let violation = check(
            "script-src-attr 'none'; script-src 'unsafe-inline'",
            "run()",
        )
        .expect("script-src-attr should take precedence over script-src");
        assert_eq!(violation.effective_directive, "script-src-attr");
        assert_eq!(violation.blocked_uri, "inline");
        assert!(check("script-src 'unsafe-inline'; default-src 'none'", "run()").is_none());
        assert!(check("default-src 'unsafe-inline'", "run()").is_none());
    }

    #[test]
    fn inline_event_handler_hash_requires_unsafe_hashes_and_an_exact_digest() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        let source = "t1.done();";
        let matching_hash = "'sha256-wmuLCpoj8EMqfQlPnt5NIMgKkCK62CxAkAiewI0zZps='";
        let check = |policy: &str, source: &str| {
            content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
                source,
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
        };

        assert!(
            check(
                &format!("script-src 'unsafe-hashes' {matching_hash}"),
                source,
            )
            .is_none()
        );
        assert!(check(&format!("script-src {matching_hash}"), source).is_some());
        assert!(
            check(
                &format!("script-src 'unsafe-hashes' {matching_hash}"),
                "t1.done()",
            )
            .is_some()
        );
    }

    #[test]
    fn inline_navigation_hashes_the_complete_javascript_url() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        let source = "javascript:opener.postMessage('pass', '*')";
        let matching_hash = "'sha256-IIiAJ8UuliU8o1qAv6CV4P3R8DeTf/v3MrsCwXW171Y='";
        let check = |policy: &str, source: &str| {
            content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                policy,
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::DocumentInlineNavigation,
                source,
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
        };

        assert!(
            check(
                &format!("script-src 'unsafe-hashes' {matching_hash}"),
                source,
            )
            .is_none()
        );
        assert!(check(&format!("script-src {matching_hash}"), source).is_some());
        let violation = check(
            &format!("script-src-elem 'none'; script-src 'unsafe-hashes' {matching_hash}"),
            source,
        )
        .expect("script-src-elem should take precedence for inline navigation");
        assert_eq!(violation.effective_directive, "script-src-elem");
        assert_eq!(violation.blocked_uri, "inline");
    }

    #[test]
    fn inline_event_handler_unsafe_inline_is_ignored_when_a_nonce_or_hash_is_present() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        for policy in [
            "script-src 'unsafe-inline' 'nonce-abc'",
            "script-src 'unsafe-inline' 'sha256-wmuLCpoj8EMqfQlPnt5NIMgKkCK62CxAkAiewI0zZps='",
        ] {
            assert!(
                content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                    policy,
                    &protected_url(),
                    ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
                    "untrusted()",
                    ContentSecurityPolicyDisposition::Enforce,
                    &reporting_endpoints,
                )
                .is_some(),
                "{policy}",
            );
        }
    }

    #[test]
    fn non_url_wasm_eval_uses_script_src_or_default_src() {
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
        for policy in [
            "script-src 'wasm-unsafe-eval'; default-src 'none'",
            "script-src 'unsafe-eval'",
            "default-src 'unsafe-eval'",
        ] {
            assert!(
                content_security_policy_non_url_violation_with_source(
                    policy,
                    &protected_url(),
                    ContentSecurityPolicyNonUrlKind::WasmEval,
                    None,
                    ContentSecurityPolicyDisposition::Enforce,
                    &reporting_endpoints,
                )
                .is_none(),
                "{policy}"
            );
        }

        let violation = content_security_policy_non_url_violation_with_source(
            "default-src 'self'",
            &protected_url(),
            ContentSecurityPolicyNonUrlKind::WasmEval,
            None,
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
        .expect("default-src should block wasm eval without unsafe eval source");
        assert_eq!(violation.effective_directive, "script-src");
        assert_eq!(violation.blocked_uri, "wasm-eval");
    }

    #[test]
    fn trusted_types_directive_filters_policy_names() {
        let allowed = |policies: &[&str], name: &str, is_duplicate: bool| {
            policies
                .iter()
                .all(|policy| policy_allows_trusted_type_policy_name(policy, name, is_duplicate))
        };

        assert!(allowed(&[], "SomeName", false));
        assert!(allowed(&[], "SomeName", true));
        assert!(allowed(&["default-src 'none'"], "SomeName", false));
        assert!(allowed(
            &["trusted-types SomeName OtherName"],
            "SomeName",
            false
        ));
        assert!(allowed(
            &["trusted-types * 'aLLow-dUPLIcates'"],
            "SomeName",
            true
        ));
        assert!(allowed(
            &["trusted-types 'none' SomeName"],
            "SomeName",
            false
        ));
        assert!(!allowed(&["trusted-types"], "SomeName", false));
        assert!(!allowed(&["trusted-types 'nONe'"], "SomeName", false));
        assert!(!allowed(&["trusted-types SomeName"], "SomeName", true));
        assert!(!allowed(&["trusted-types SomeName"], "default", false));
        assert!(!allowed(
            &["trusted-types SomeName", "trusted-types OtherName"],
            "SomeName",
            false
        ));
    }

    #[test]
    fn trusted_types_eval_requires_enforcement_and_an_operative_keyword() {
        let allowed = |policies: &[&str]| {
            content_security_policy_allows_trusted_types_eval(
                &policies
                    .iter()
                    .map(|policy| policy.to_string())
                    .collect::<Vec<_>>(),
            )
        };

        assert!(allowed(&[
            "require-trusted-types-for 'script'; script-src 'trusted-types-eval'"
        ]));
        assert!(allowed(&[
            "require-trusted-types-for 'script'",
            "default-src 'trusted-types-eval'"
        ]));
        assert!(!allowed(&["script-src 'trusted-types-eval'"]));
        assert!(!allowed(&[
            "require-trusted-types-for 'script'; default-src 'trusted-types-eval'; script-src 'self'"
        ]));
        assert!(!allowed(&[
            "require-trusted-types-for 'script'; script-src 'unsafe-eval'"
        ]));
    }

    #[test]
    fn sandbox_directive_forces_opaque_origin_without_allow_same_origin() {
        let forces_opaque = |policies: &[&str]| {
            content_security_policy_forces_opaque_origin(
                &policies
                    .iter()
                    .map(|policy| policy.to_string())
                    .collect::<Vec<_>>(),
            )
        };

        assert!(!forces_opaque(&[]));
        assert!(!forces_opaque(&["script-src 'self'"]));
        assert!(forces_opaque(&["sandbox"]));
        assert!(forces_opaque(&["sandbox allow-scripts"]));
        assert!(!forces_opaque(&["sandbox allow-scripts allow-same-origin"]));
        assert!(!forces_opaque(&["sandbox allow-scripts ALLOW-SAME-ORIGIN"]));
    }

    #[test]
    fn sandbox_directive_sandboxes_document_domain_even_with_allow_same_origin() {
        let sandboxes_document_domain = |policies: &[&str]| {
            content_security_policy_sandboxes_document_domain(
                &policies
                    .iter()
                    .map(|policy| policy.to_string())
                    .collect::<Vec<_>>(),
            )
        };

        assert!(!sandboxes_document_domain(&[]));
        assert!(!sandboxes_document_domain(&["script-src 'self'"]));
        assert!(sandboxes_document_domain(&["sandbox"]));
        assert!(sandboxes_document_domain(&["sandbox allow-scripts"]));
        assert!(sandboxes_document_domain(&[
            "sandbox allow-scripts allow-same-origin"
        ]));
        assert!(sandboxes_document_domain(&[
            "sandbox allow-scripts ALLOW-SAME-ORIGIN"
        ]));
    }

    #[test]
    fn sandbox_directive_allows_scripts_only_when_all_active_sandboxes_allow_it() {
        let allows_scripts = |policies: &[&str]| {
            content_security_policy_sandbox_allows_scripts(
                &policies
                    .iter()
                    .map(|policy| policy.to_string())
                    .collect::<Vec<_>>(),
            )
        };

        assert!(allows_scripts(&[]));
        assert!(allows_scripts(&["script-src 'self'"]));
        assert!(!allows_scripts(&["sandbox"]));
        assert!(allows_scripts(&["sandbox allow-scripts"]));
        assert!(allows_scripts(&["sandbox ALLOW-SCRIPTS"]));
        assert!(!allows_scripts(&[
            "sandbox allow-scripts",
            "sandbox allow-same-origin"
        ]));
    }

    #[test]
    fn sandbox_directive_allows_modals_only_when_all_active_sandboxes_allow_it() {
        let allows_modals = |policies: &[&str]| {
            content_security_policy_sandbox_allows_modals(
                &policies
                    .iter()
                    .map(|policy| policy.to_string())
                    .collect::<Vec<_>>(),
            )
        };

        assert!(allows_modals(&[]));
        assert!(allows_modals(&["script-src 'self'"]));
        assert!(!allows_modals(&["sandbox"]));
        assert!(allows_modals(&["sandbox allow-modals"]));
        assert!(allows_modals(&["sandbox ALLOW-MODALS"]));
        assert!(!allows_modals(&[
            "sandbox allow-modals",
            "sandbox allow-scripts"
        ]));
    }

    #[test]
    fn sandbox_directive_allows_popups_to_escape_only_when_all_active_sandboxes_allow_it() {
        let allows_popups_to_escape = |policies: &[&str]| {
            content_security_policy_sandbox_allows_popups_to_escape(
                &policies
                    .iter()
                    .map(|policy| policy.to_string())
                    .collect::<Vec<_>>(),
            )
        };

        assert!(!allows_popups_to_escape(&[]));
        assert!(!allows_popups_to_escape(&["script-src 'self'"]));
        assert!(!allows_popups_to_escape(&["sandbox"]));
        assert!(allows_popups_to_escape(&[
            "sandbox allow-popups-to-escape-sandbox"
        ]));
        assert!(allows_popups_to_escape(&[
            "sandbox ALLOW-POPUPS-TO-ESCAPE-SANDBOX"
        ]));
        assert!(!allows_popups_to_escape(&[
            "sandbox allow-popups-to-escape-sandbox",
            "sandbox allow-scripts"
        ]));
    }
}
