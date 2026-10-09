//! Connect the standalone CSP engine to V8 events and the renderer loader.

pub(crate) use moli_csp::*;

use crate::context_bootstrap::{initialize_event_object, mark_event_trusted};
use crate::network::ResourceRequestClient;
use serde_json::json;
use std::{collections::{HashSet, hash_map::DefaultHasher}, hash::{Hash, Hasher}};

use moli_fetch::{
    Request, RequestCredentialsMode, RequestMode, RequestRedirectMode, RequestResourceType,
};

pub(crate) fn create_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let event = crate::context_bootstrap::new_event_state(scope);
    initialize_event_object(scope, event, "securitypolicyviolation", true, false);
    event.set(
        scope,
        crate::util::v8str(scope, "composed").into(),
        v8::Boolean::new(scope, true).into(),
    )?;
    crate::context_bootstrap::initialize_native_security_policy_violation_event(
        scope, event, fields,
    )?;
    mark_event_trusted(scope, event);
    crate::context_bootstrap::new_event_wrapper(scope, event)
}

/// Network report history for one Document or WorkerGlobalScope. DOM events
/// remain independent: every violation still dispatches an event.
#[derive(Default)]
pub(crate) struct ContentSecurityPolicyReports {
    sent: parking_lot::Mutex<HashSet<u64>>,
}

impl ContentSecurityPolicyReports {
    pub(crate) fn requests(
        &self,
        fields: &ContentSecurityPolicyViolationEventFields<'_>,
        report_uri_endpoints: &[String],
        report_to_endpoints: &[String],
    ) -> Vec<Request> {
        if report_uri_endpoints.is_empty() && report_to_endpoints.is_empty() {
            return Vec::new();
        }
        let body = content_security_policy_violation_report_body(fields);
        // Keep a compact fingerprint rather than retaining every report body.
        // Claim it before dispatch so overlapping requests share the history.
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        if !self.sent.lock().insert(hasher.finish()) {
            return Vec::new();
        }
        let mut requests = Vec::new();
        append_content_security_policy_report_requests(
            &mut requests,
            &body,
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
            Some(bytes::Bytes::copy_from_slice(body.as_bytes())),
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
    reports: &ContentSecurityPolicyReports,
    loader: &ResourceRequestClient,
    origin: moli_url::WebOrigin,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
    report_uri_endpoints: &[String],
    report_to_endpoints: &[String],
) {
    for request in reports.requests(fields, report_uri_endpoints, report_to_endpoints) {
        send_content_security_policy_report_request(
            loader,
            request.with_request_origin(origin.clone()),
        );
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

/// The script location available when a CSP violation is created.
#[derive(Clone, Debug, Default)]
pub(crate) struct ContentSecurityPolicySourceLocation(Option<(String, i32, i32)>);

impl ContentSecurityPolicySourceLocation {
    pub(crate) fn capture(scope: &mut v8::PinScope<'_, '_>) -> Self {
        Self(current_script_violation_location(scope))
    }

    pub(crate) fn apply_to(&self, violation: &mut ContentSecurityPolicyUrlViolation) {
        if let Some((source_file, line_number, column_number)) = &self.0 {
            violation.source_file.clone_from(source_file);
            violation.line_number = *line_number;
            violation.column_number = *column_number;
        } else {
            violation.source_file.clear();
            violation.line_number = 0;
            violation.column_number = 0;
        }
    }
}

pub(crate) fn current_script_violation_location(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<(String, i32, i32)> {
    let stack = v8::StackTrace::current_stack_trace(scope, 1)?;
    let frame = stack.get_frame(scope, 0)?;
    // Imported worker scripts retain the originally requested URL separately
    // from their final resource name and their (possibly muted) import base.
    // Reporting the final URL could disclose a cross-origin redirect target.
    let source_file = scope
        .get_current_host_defined_options()
        .and_then(|options| {
            crate::util::script_request_url_from_host_defined_options(scope, options)
        })
        .map(|url| url.to_string())
        .or_else(|| {
            frame
                .get_script_name_or_source_url(scope)
                .map(|source| source.to_rust_string_lossy(scope))
        })
        .map(|source| content_security_policy_source_file_for_report(&source))
        .unwrap_or_default();
    let line_number = i32::try_from(frame.get_line_number())
        .unwrap_or_default()
        .max(0);
    let column_number = i32::try_from(frame.get_column()).unwrap_or_default().max(0);
    Some((source_file, line_number, column_number))
}

pub(crate) fn notify_content_security_policy_reporting_observers(
    scope: &mut v8::PinScope<'_, '_>,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
) {
    crate::context_bootstrap::notify_reporting_observers(
        scope,
        "csp-violation",
        fields.document_uri,
        json!({
            "documentURL": fields.document_uri,
            "referrer": fields.referrer,
            "blockedURL": fields.blocked_uri,
            "effectiveDirective": fields.effective_directive,
            "originalPolicy": fields.original_policy,
            "sourceFile": (!fields.source_file.is_empty()).then_some(fields.source_file),
            "sample": fields.sample,
            "disposition": fields.disposition.as_str(),
            "statusCode": fields.status_code,
            "lineNumber": (fields.line_number != 0).then_some(fields.line_number),
            "columnNumber": (fields.column_number != 0).then_some(fields.column_number),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn protected_url() -> Url {
        Url::parse("https://app.test/page.html").unwrap()
    }
    fn request_url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    #[test]
    fn violation_report_request_uses_csp_fetch_security_modes() {
        let violation = content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
            &["connect-src 'none'; report-uri /csp-report".into()],
            &protected_url(),
            &request_url("https://api.test/data.json"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
        )
        .expect("blocked URL should produce violation");
        let requests = ContentSecurityPolicyReports::default().requests(
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
        assert_eq!(requests[0].redirect_mode(), RequestRedirectMode::Error);
    }
    #[test]
    fn duplicate_reports_preserve_distinct_violations_and_all_endpoints() {
        let violation = content_security_policy_url_violation_with_redirect_status(
            &["connect-src 'none'; report-uri /csp-report".into()],
            &protected_url(),
            &request_url("https://api.test/data.json"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .unwrap();
        let endpoints = vec![
            "https://app.test/report-one".to_owned(),
            "https://app.test/report-two".to_owned(),
        ];
        for use_reporting_api in [false, true] {
            let reports = ContentSecurityPolicyReports::default();
            let (legacy, reporting) = if use_reporting_api {
                (&[][..], endpoints.as_slice())
            } else {
                (endpoints.as_slice(), &[][..])
            };
            let assert_new_report = |fields: &ContentSecurityPolicyViolationEventFields<'_>| {
                let requests = reports.requests(fields, legacy, reporting);
                assert_eq!(
                    requests.len(),
                    2,
                    "every endpoint receives a distinct report"
                );
                assert_eq!(requests[0].url.as_str(), endpoints[0]);
                assert_eq!(requests[1].url.as_str(), endpoints[1]);
                assert!(reports.requests(fields, legacy, reporting).is_empty());
            };
            let mut fields =
                ContentSecurityPolicyViolationEventFields::from_url_violation(&violation);
            assert_new_report(&fields);
            fields.line_number = 10;
            assert_new_report(&fields);
            fields.column_number = 20;
            assert_new_report(&fields);
            fields.sample = "different script";
            assert_new_report(&fields);
            fields.blocked_uri = "https://api.test/other.json";
            assert_new_report(&fields);
            fields.disposition = ContentSecurityPolicyDisposition::Report;
            assert_new_report(&fields);
            fields.original_policy = "connect-src https://other.test; report-uri /csp-report";
            assert_new_report(&fields);
        }
    }

    #[test]
    fn non_url_reports_strip_credentials_and_fragments_before_deduplication() {
        let reports = ContentSecurityPolicyReports::default();
        let endpoints = ContentSecurityPolicyReportingEndpoints::default();
        for (fragment, expected_count) in [("first", 1), ("second", 0)] {
            let url = Url::parse(&format!(
                "https://user:password@app.test/page?query=kept#{fragment}"
            ))
            .unwrap();
            let violations = [
                content_security_policy_non_url_violation_with_source(
                    &ContentSecurityPolicy::from("script-src 'self'; report-uri /report"), &url, ContentSecurityPolicyNonUrlKind::Eval,
                    Some("blocked()"), ContentSecurityPolicyDisposition::Enforce, &endpoints,
                ),
                content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                    &ContentSecurityPolicy::from("script-src 'self'; report-uri /report"), &url, ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
                    "blocked()", ContentSecurityPolicyDisposition::Enforce, &endpoints,
                ),
                content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints(
                    &ContentSecurityPolicy::from("require-trusted-types-for 'script'; report-uri /report"), &url, "eval", "blocked()",
                    ContentSecurityPolicyDisposition::Enforce, &endpoints,
                ),
            ];
            for violation in violations {
                let violation = violation.unwrap();
                assert_eq!(violation.document_uri, "https://app.test/page?query=kept");
                assert_eq!(
                    reports
                        .requests(
                            &ContentSecurityPolicyViolationEventFields::from_url_violation(
                                &violation
                            ),
                            &violation.report_uri_endpoints,
                            &[],
                        )
                        .len(),
                    expected_count
                );
            }
        }
    }

    #[test]
    fn violation_report_formats_preserve_captured_script_locations() {
        let mut violation = content_security_policy_url_violation_with_redirect_status(
            &["connect-src 'none'".into()],
            &protected_url(),
            &request_url("https://api.test/data.json"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .unwrap();
        violation.source_file = "https://app.test/imported/script.js".into();
        violation.line_number = 12;
        violation.column_number = 34;
        let fields = ContentSecurityPolicyViolationEventFields::from_url_violation(&violation);
        assert_eq!(fields.source_file, violation.source_file);
        assert_eq!((fields.line_number, fields.column_number), (12, 34));
        let legacy: serde_json::Value =
            serde_json::from_str(&content_security_policy_violation_report_body(&fields)).unwrap();
        assert_eq!(legacy["csp-report"]["source-file"], violation.source_file);
        assert_eq!(legacy["csp-report"]["line-number"], 12);
        assert_eq!(legacy["csp-report"]["column-number"], 34);
        let reporting: serde_json::Value =
            serde_json::from_str(&content_security_policy_reporting_api_report_body(&fields))
                .unwrap();
        assert_eq!(reporting[0]["body"]["sourceFile"], violation.source_file);
        assert_eq!(reporting[0]["body"]["lineNumber"], 12);
        assert_eq!(reporting[0]["body"]["columnNumber"], 34);
    
        ContentSecurityPolicySourceLocation::default().apply_to(&mut violation);
        let fields = ContentSecurityPolicyViolationEventFields::from_url_violation(&violation);
        let legacy: serde_json::Value =
            serde_json::from_str(&content_security_policy_violation_report_body(&fields)).unwrap();
        for key in ["source-file", "line-number", "column-number"] {
            assert!(legacy["csp-report"].get(key).is_none());
        }
        let reporting: serde_json::Value =
            serde_json::from_str(&content_security_policy_reporting_api_report_body(&fields))
                .unwrap();
        for key in ["sourceFile", "lineNumber", "columnNumber"] {
            assert!(reporting[0]["body"].get(key).is_none());
        }
    }

}
