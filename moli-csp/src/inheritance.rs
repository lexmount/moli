use super::{
    ContentSecurityPolicy, ContentSecurityPolicyDisposition,
    ContentSecurityPolicyReportingEndpoints,
};
use url::Url;

/// The source and delivery rules survive cloning a policy into a local Worker.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InheritedContentSecurityPolicy {
    pub self_url: Option<Url>,
    pub header_policies: Vec<ContentSecurityPolicy>,
    pub meta_policies: Vec<ContentSecurityPolicy>,
    pub report_only_policies: Vec<ContentSecurityPolicy>,
    pub reporting_endpoints: ContentSecurityPolicyReportingEndpoints,
}

impl InheritedContentSecurityPolicy {
    pub fn policies(
        &self,
        disposition: ContentSecurityPolicyDisposition,
    ) -> impl Iterator<Item = (&ContentSecurityPolicy, bool)> {
        let (headers, meta) = match disposition {
            ContentSecurityPolicyDisposition::Enforce => (
                self.header_policies.as_slice(),
                self.meta_policies.as_slice(),
            ),
            ContentSecurityPolicyDisposition::Report => {
                (self.report_only_policies.as_slice(), &[][..])
            }
        };
        headers
            .iter()
            .map(|policy| (policy, true))
            .chain(meta.iter().map(|policy| (policy, false)))
    }

    pub fn enforced_policies(&self) -> Vec<ContentSecurityPolicy> {
        self.policies(ContentSecurityPolicyDisposition::Enforce)
            .map(|(policy, _)| policy.to_owned())
            .collect()
    }

    pub fn url_violation(
        &self,
        protected_url: &Url,
        request_url: &Url,
        kind: super::ContentSecurityPolicyResourceKind,
        redirect_status: super::ContentSecurityPolicyRedirectStatus,
        disposition: ContentSecurityPolicyDisposition,
    ) -> Option<super::ContentSecurityPolicyUrlViolation> {
        self.policies(disposition).find_map(|(policy, report_uri_enabled)| {
            let mut violation = super::content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
                std::slice::from_ref(policy),
                self.self_url.as_ref().unwrap_or(protected_url),
                request_url,
                kind,
                redirect_status,
                disposition,
                &self.reporting_endpoints,
            )?;
            violation.document_uri = super::csp_url_for_report(protected_url);
            violation.source_file = super::csp_url_for_report(protected_url);
            if !report_uri_enabled {
                violation.report_uri_endpoints.clear();
            }
            Some(violation)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyResourceKind};

    #[test]
    fn inherited_worker_policy_preserves_self_origin_delivery_and_report_url() {
        let source_url = Url::parse("https://example.test/creator/page.html").unwrap();
        let policy_text = "connect-src 'self'; report-uri ./report".to_owned();
        for scheme in ["blob:https://example.test/worker", "data:text/javascript,"] {
            let worker_url = Url::parse(scheme).unwrap();
            for disposition in [
                ContentSecurityPolicyDisposition::Enforce,
                ContentSecurityPolicyDisposition::Report,
            ] {
                for is_meta in [false, true] {
                    if is_meta && disposition == ContentSecurityPolicyDisposition::Report {
                        continue;
                    }
                    let mut policy = InheritedContentSecurityPolicy {
                        self_url: Some(source_url.clone()),
                        ..Default::default()
                    };
                    match (disposition, is_meta) {
                        (ContentSecurityPolicyDisposition::Report, _) => {
                            policy.report_only_policies.push(policy_text.clone().into())
                        }
                        (_, true) => policy.meta_policies.push(policy_text.clone().into()),
                        (_, false) => policy.header_policies.push(policy_text.clone().into()),
                    }
                    let check = |request: &str| {
                        policy.url_violation(
                            &worker_url,
                            &Url::parse(request).unwrap(),
                            ContentSecurityPolicyResourceKind::WorkerConnect,
                            ContentSecurityPolicyRedirectStatus::NoRedirect,
                            disposition,
                        )
                    };
                    assert!(check("https://example.test/allowed").is_none());
                    let violation = check("https://cross.test/blocked").unwrap();
                    assert_eq!(violation.document_uri, worker_url.scheme());
                    assert_eq!(violation.source_file, worker_url.scheme());
                    assert_eq!(violation.original_policy, policy_text);
                    assert_eq!(violation.disposition, disposition);
                    if is_meta {
                        assert!(violation.report_uri_endpoints.is_empty());
                    } else {
                        assert_eq!(
                            violation.report_uri_endpoints,
                            vec!["https://example.test/creator/report"]
                        );
                    }
                }
            }
        }
    }
}
