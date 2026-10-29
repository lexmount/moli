//! Connect the standalone CSP engine to V8 events and the renderer loader.

pub(crate) use moli_csp::*;

use crate::context_bootstrap::{initialize_event_object, mark_event_trusted};
use crate::network::ResourceRequestClient;
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
    loader: &ResourceRequestClient,
    origin: moli_url::WebOrigin,
    fields: &ContentSecurityPolicyViolationEventFields<'_>,
    report_uri_endpoints: &[String],
    report_to_endpoints: &[String],
) {
    for request in
        content_security_policy_report_requests(fields, report_uri_endpoints, report_to_endpoints)
    {
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

pub(crate) fn current_script_violation_location(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<(String, i32, i32)> {
    let stack = v8::StackTrace::current_stack_trace(scope, 1)?;
    let frame = stack.get_frame(scope, 0)?;
    let source_file = frame
        .get_script_name_or_source_url(scope)
        .map(|source| source.to_rust_string_lossy(scope))
        .map(|source| {
            crate::content_security_policy::content_security_policy_source_file_for_report(&source)
        })
        .unwrap_or_default();
    let line_number = i32::try_from(frame.get_line_number())
        .unwrap_or_default()
        .max(0);
    let column_number = i32::try_from(frame.get_column()).unwrap_or_default().max(0);
    Some((source_file, line_number, column_number))
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
        assert_eq!(requests[0].redirect_mode(), RequestRedirectMode::Error);
    }
}
