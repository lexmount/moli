use super::super::JsContextHost;
use super::super::child_documents::ChildDocumentNavigationInitiator;
use crate::{
    content_security_policy::IframeCspAttribute,
    document_runtime::{DocumentPolicyContainer, DomHandle},
};
use url::Url;

/// Policy inputs belong to the navigation admission, not to the later fetch
/// task or a subsequent refresh of the owner element's live attributes.
#[derive(Clone, Debug, Default)]
pub(in crate::native_bridge::context_host) struct ChildNavigationTargetSnapshot {
    required_csp: Option<IframeCspAttribute>,
    embedding_origin: Option<String>,
    owner_referrer_policy: Option<String>,
    creator_referrer_policy: Option<String>,
}

impl ChildNavigationTargetSnapshot {
    pub(in crate::native_bridge::context_host) fn referrer_policy<'a>(
        &'a self,
        initiator: ChildDocumentNavigationInitiator,
        source_referrer_policy: Option<&'a str>,
    ) -> &'a str {
        let owner_policy = if initiator == ChildDocumentNavigationInitiator::FrameOwnerElement {
            if self.embedding_origin.as_deref() == Some("null") {
                return "no-referrer";
            }
            self.owner_referrer_policy.as_deref()
        } else {
            None
        };
        let source_policy = source_referrer_policy.or(self.creator_referrer_policy.as_deref());
        moli_fetch::effective_referrer_policy(owner_policy, source_policy)
    }

    pub(in crate::native_bridge::context_host) fn apply_response_policy(
        &self,
        policy: &mut DocumentPolicyContainer,
        final_url: &Url,
        headers: &[(String, Vec<u8>)],
    ) -> bool {
        self.required_csp.as_ref().is_none_or(|required| {
            policy.apply_required_content_security_policy(
                required,
                self.embedding_origin.as_deref().unwrap_or("null"),
                final_url,
                headers,
            )
        })
    }

    pub(in crate::native_bridge::context_host) fn configure_request(
        &self,
        mut request: moli_fetch::Request,
        initiator: ChildDocumentNavigationInitiator,
        source_referrer_policy: Option<&str>,
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
        let metadata = request
            .subresource_request_metadata()
            .cloned()
            .unwrap_or_default();
        request.with_subresource_request_metadata(moli_fetch::SubresourceRequestMetadata {
            referrer_policy: Some(
                self.referrer_policy(initiator, source_referrer_policy)
                    .to_owned(),
            ),
            ..metadata
        })
    }
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn snapshot_child_navigation_target(
        &self,
        handle: DomHandle,
        target_url: &Url,
    ) -> ChildNavigationTargetSnapshot {
        let owner = self.owner_dispatch_scope_for_node(handle);
        let embedding_origin = owner
            .and_then(|owner| self.window_access_origin_for_dispatch_scope(owner))
            .map(|origin| origin.serialized_origin());
        let creator_policy =
            owner.and_then(|owner| self.document_policy_container_for_inheritance(owner));
        let creator_referrer_policy = creator_policy
            .as_ref()
            .and_then(|policy| policy.referrer_policy.clone());
        let is_iframe = self.dom_host().is_html_element_named(handle, "iframe");
        let owner_referrer_policy = if is_iframe {
            self.dom_host()
                .get_attribute(handle, "referrerpolicy")
                .as_deref()
                .map(crate::native_bridge::element::canonical_referrer_policy_value)
                .filter(|policy| !policy.is_empty())
                .map(ToOwned::to_owned)
        } else {
            None
        };
        let snapshot = ChildNavigationTargetSnapshot {
            embedding_origin,
            owner_referrer_policy,
            creator_referrer_policy,
            ..Default::default()
        };
        if self.bypass_content_security_policy() {
            return snapshot;
        }
        let inherited = creator_policy.and_then(|policy| policy.required_content_security_policy);
        let required_csp = if is_iframe {
            self.dom_host()
                .get_attribute(handle, "csp")
                .as_deref()
                .and_then(IframeCspAttribute::parse)
        } else {
            None
        };
        let required_csp = required_csp
            .filter(|policy| {
                inherited.as_ref().is_none_or(|parent| {
                    parent.subsumes(std::slice::from_ref(policy.policy()), target_url)
                })
            })
            .or(inherited);
        ChildNavigationTargetSnapshot {
            required_csp,
            ..snapshot
        }
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
                ..Default::default()
            };
            let request = snapshot.configure_request(
                request,
                ChildDocumentNavigationInitiator::FrameOwnerElement,
                None,
            );
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
