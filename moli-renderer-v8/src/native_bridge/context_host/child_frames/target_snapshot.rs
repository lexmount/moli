use super::super::JsContextHost;
use crate::{content_security_policy::IframeCspAttribute, document_runtime::DomHandle};

/// Policy inputs belong to the navigation admission, not to the later fetch
/// task or a subsequent refresh of the owner element's live attributes.
#[derive(Clone, Debug, Default)]
pub(in crate::native_bridge::context_host) struct ChildNavigationTargetSnapshot {
    required_csp: Option<IframeCspAttribute>,
}

impl ChildNavigationTargetSnapshot {
    pub(in crate::native_bridge::context_host) fn configure_request(
        &self,
        mut request: moli_fetch::Request,
    ) -> moli_fetch::Request {
        // A reused request cannot supply its own requirement or retain a
        // previous target's header. There is at most one authoritative value.
        request.request_headers.remove_all("Sec-Required-CSP");
        if let Some(policy) = &self.required_csp {
            request.request_headers.push((
                "Sec-Required-CSP".to_owned(),
                policy.as_str().as_bytes().to_vec(),
            ));
        }
        request
    }
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn snapshot_child_navigation_target(
        &self,
        handle: DomHandle,
    ) -> ChildNavigationTargetSnapshot {
        let required_csp = if self.dom_host().is_html_element_named(handle, "iframe") {
            self.dom_host()
                .get_attribute(handle, "csp")
                .as_deref()
                .and_then(IframeCspAttribute::parse)
        } else {
            None
        };
        ChildNavigationTargetSnapshot { required_csp }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_target_owns_the_required_csp_header_and_preserves_other_raw_fields() {
        for policy in [
            None,
            IframeCspAttribute::parse("script-src 'unsafe-inline'"),
        ] {
            let mut request = moli_fetch::Request::get_with_url(
                url::Url::parse("https://embedee.test/").unwrap(),
            );
            request.request_headers = moli_fetch::RequestHeaders::from_bytes(vec![
                ("Sec-Required-CSP".into(), b"forged".to_vec()),
                ("Cookie".into(), vec![0xe9]),
                ("sec-required-csp".into(), b"duplicate".to_vec()),
                ("Sec-Required-CSP-Other".into(), b"kept".to_vec()),
            ]);
            let snapshot = ChildNavigationTargetSnapshot {
                required_csp: policy,
            };
            let request = snapshot.configure_request(request);
            let mut expected = vec![
                ("Cookie".into(), vec![0xe9]),
                ("Sec-Required-CSP-Other".into(), b"kept".to_vec()),
            ];
            if snapshot.required_csp.is_some() {
                expected.push((
                    "Sec-Required-CSP".into(),
                    b"script-src 'unsafe-inline'".to_vec(),
                ));
            }
            assert_eq!(
                request.request_headers,
                moli_fetch::RequestHeaders::from_bytes(expected)
            );
        }
    }
}
