use super::*;

use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyNonUrlKind,
    ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyReportingEndpoints,
    ContentSecurityPolicyResourceKind, ContentSecurityPolicyScriptElementRequest,
    ContentSecurityPolicyStyleElementRequest, ContentSecurityPolicyUrlViolation,
    ContentSecurityPolicyViolationEventFields, TrustedTypesForScriptRequirements,
    content_security_policy_allows_trusted_type_policy_name,
    content_security_policy_allows_trusted_types_eval, content_security_policy_headers,
    content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_non_url_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_report_only_headers,
    content_security_policy_requires_trusted_types_for_script,
    content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request,
    content_security_policy_style_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request,
    content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints,
    create_security_policy_violation_event,
};
use crate::util::v8str;

pub(crate) type DocumentContentSecurityPolicyViolation = ContentSecurityPolicyUrlViolation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentSubresourceCspKind {
    Image,
    Manifest,
    Media,
}

impl DocumentSubresourceCspKind {
    fn resource_kind(self) -> ContentSecurityPolicyResourceKind {
        match self {
            Self::Image => ContentSecurityPolicyResourceKind::DocumentImage,
            Self::Manifest => ContentSecurityPolicyResourceKind::DocumentManifest,
            Self::Media => ContentSecurityPolicyResourceKind::DocumentMedia,
        }
    }
}

#[must_use = "report-only and enforced CSP results must be handled together"]
pub(crate) struct DocumentContentSecurityPolicyCheck {
    report_only_violation: Option<DocumentContentSecurityPolicyViolation>,
    enforced_violation: Option<DocumentContentSecurityPolicyViolation>,
}

impl DocumentContentSecurityPolicyCheck {
    pub(crate) fn into_violations(
        self,
    ) -> (
        Option<DocumentContentSecurityPolicyViolation>,
        Option<DocumentContentSecurityPolicyViolation>,
    ) {
        (self.report_only_violation, self.enforced_violation)
    }

    #[cfg(test)]
    fn report_only_violation(&self) -> Option<&DocumentContentSecurityPolicyViolation> {
        self.report_only_violation.as_ref()
    }

    #[cfg(test)]
    fn enforced_violation(&self) -> Option<&DocumentContentSecurityPolicyViolation> {
        self.enforced_violation.as_ref()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DocumentConnectPolicySnapshot {
    enforce_policies: Vec<String>,
    report_only_policies: Vec<String>,
    reporting_endpoints: ContentSecurityPolicyReportingEndpoints,
}

impl DocumentConnectPolicySnapshot {
    pub(crate) fn from_policy_container(policy: &DocumentPolicyContainer) -> Self {
        let mut enforce_policies = policy.response_content_security_policies.clone();
        enforce_policies.extend(policy.document_content_security_policies.iter().cloned());
        Self {
            enforce_policies,
            report_only_policies: policy
                .response_content_security_report_only_policies
                .clone(),
            reporting_endpoints: policy.content_security_reporting_endpoints.clone(),
        }
    }

    pub(crate) fn enforce_violation(
        &self,
        document_url: &Url,
        request_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        document_connect_policy_violation(
            &self.enforce_policies,
            &self.reporting_endpoints,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn report_only_violation(
        &self,
        document_url: &Url,
        request_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        document_connect_policy_violation(
            &self.report_only_policies,
            &self.reporting_endpoints,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Report,
        )
    }
}

impl DocumentPolicyContainer {
    pub(crate) fn clear_content_security_policy_for_bypass(&mut self) {
        self.document_content_security_policies.clear();
        self.response_content_security_policies.clear();
        self.response_content_security_report_only_policies.clear();
        self.content_security_reporting_endpoints = Default::default();
        self.sandbox = DocumentSandboxPolicy::default();
    }
}

impl DocumentRuntime {
    pub(super) fn apply_style_csp_mutation_followups<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        attribute_mutations: &[super::mutation_commands::InlineStyleAttributeCspMutation],
        connected_roots: &[DomHandle],
        stylesheet_owner_changes: &[crate::dom::native::DomStylesheetOwnerChange],
    ) {
        let mut checked_attributes = HashSet::new();
        for mutation in attribute_mutations {
            let target = mutation.target();
            checked_attributes.insert(target);
            self.apply_inline_style_attribute_csp_check(
                scope,
                host_ptr,
                target,
                mutation.new_value(),
            );
        }

        let mut stylesheet_owners = Vec::new();
        for change in stylesheet_owner_changes {
            let owner = change.owner();
            let is_inline_style = self.dom_host.is_inline_style_sheet_owner(owner);
            let is_link = self.dom_host.is_html_element_named(owner, "link");
            let reprocesses_owner = match change.kind() {
                DomStylesheetOwnerChangeKind::Registered
                | DomStylesheetOwnerChangeKind::OwnerDocumentChanged
                | DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: true } => true,
                DomStylesheetOwnerChangeKind::Contents => is_inline_style,
                DomStylesheetOwnerChangeKind::Attribute {
                    namespace,
                    local_name,
                } => {
                    namespace.is_none()
                        && ((is_inline_style && local_name == "type")
                            || (is_link && attribute_reprocesses_connected_stylesheet(local_name)))
                }
                DomStylesheetOwnerChangeKind::Unregistered
                | DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: false } => {
                    self.set_stylesheet_owner_csp_disposition(
                        host_ptr,
                        owner,
                        super::StylesheetOwnerCspDisposition::Allowed,
                    );
                    false
                }
            };
            if reprocesses_owner
                && (is_inline_style || is_link)
                && !stylesheet_owners.contains(&owner)
            {
                stylesheet_owners.push(owner);
            }
        }
        for &root in connected_roots {
            let mut stack = vec![root];
            while let Some(handle) = stack.pop() {
                if self.dom_host.node(handle).is_some_and(Node::is_element) {
                    if !checked_attributes.contains(&handle)
                        && unsafe { &*host_ptr }.element_inline_style_csp_state(handle)
                            == crate::style_engine::InlineStyleCspState::Unchecked
                        && let Some(source) = self.dom_host.get_attribute(handle, "style")
                    {
                        checked_attributes.insert(handle);
                        self.apply_inline_style_attribute_csp_check(
                            scope,
                            host_ptr,
                            handle,
                            Some(&source),
                        );
                    }
                    if (self.dom_host.is_inline_style_sheet_owner(handle)
                        || self.dom_host.is_html_element_named(handle, "link"))
                        && !stylesheet_owners.contains(&handle)
                    {
                        stylesheet_owners.push(handle);
                    }
                }
                if let Some(shadow_root) = self.dom_host.shadow_root_handle(handle) {
                    stack.push(shadow_root);
                }
                let mut child = self.dom_host.first_child(handle);
                while let Some(current) = child {
                    stack.push(current);
                    child = self.dom_host.next_sibling(current);
                }
            }
        }

        for owner in stylesheet_owners {
            if self.dom_host.is_inline_style_sheet_owner(owner) {
                self.apply_inline_style_element_csp_check(scope, host_ptr, owner);
            } else {
                self.apply_link_style_element_csp_check(scope, host_ptr, owner);
            }
        }
    }

    fn set_stylesheet_owner_csp_disposition(
        &mut self,
        host_ptr: *mut JsContextHost,
        owner: DomHandle,
        disposition: super::StylesheetOwnerCspDisposition,
    ) {
        self.stylesheet_lifecycle
            .owner_states
            .set_csp_disposition(owner, disposition);
        unsafe { &mut *host_ptr }
            .set_owner_style_sheet_csp_suppressed(owner, disposition.is_blocked());
    }

    pub(crate) fn stylesheet_owner_is_csp_blocked(&self, owner: DomHandle) -> bool {
        self.stylesheet_lifecycle
            .owner_states
            .csp_disposition(owner)
            .is_blocked()
    }

    fn apply_inline_style_attribute_csp_check<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        target: DomHandle,
        source: Option<&str>,
    ) {
        let Some(source) = source else {
            unsafe { &mut *host_ptr }.set_element_inline_style_csp_state(
                target,
                crate::style_engine::InlineStyleCspState::Unchecked,
            );
            return;
        };
        let (report_only_violation, enforced_violation) = self
            .inline_source_csp_check(
                ContentSecurityPolicyNonUrlKind::DocumentInlineStyleAttribute,
                source,
            )
            .into_violations();
        unsafe { &mut *host_ptr }.set_element_inline_style_csp_state(
            target,
            if enforced_violation.is_some() {
                crate::style_engine::InlineStyleCspState::BlockedAttribute
            } else {
                crate::style_engine::InlineStyleCspState::AllowedAttribute
            },
        );
        for violation in [report_only_violation, enforced_violation]
            .into_iter()
            .flatten()
        {
            self.queue_content_security_policy_violation_event_for_element_best_effort(
                scope, host_ptr, target, &violation,
            );
        }
    }

    fn apply_inline_style_element_csp_check<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        owner: DomHandle,
    ) {
        if !self.dom_host.is_connected(owner) {
            return;
        }
        let Some(element) = self.dom_host.node(owner).and_then(Node::as_element) else {
            return;
        };
        let source = self.dom_host.text_content(owner).unwrap_or_default();
        let nonce = element.cryptographic_nonce().map(str::to_owned);
        let is_declarative_css_module =
            super::stylesheet_runtime::is_declarative_css_module_style_element(element);
        let (style_report_only_violation, style_enforced_violation) = self
            .inline_style_element_csp_check(
                &source,
                ContentSecurityPolicyStyleElementRequest {
                    nonce: nonce.as_deref(),
                },
            )
            .into_violations();
        let (script_report_only_violation, script_enforced_violation) = if is_declarative_css_module
        {
            // A declarative CSS module is not a script element and carries no
            // creator-script trust to propagate through `strict-dynamic`.
            // Use the parser-inserted request shape even when script created.
            self.inline_script_element_csp_check(
                &source,
                ContentSecurityPolicyScriptElementRequest::parser_inserted_with_nonce(
                    nonce.as_deref(),
                ),
            )
            .into_violations()
        } else {
            (None, None)
        };
        self.set_stylesheet_owner_csp_disposition(
            host_ptr,
            owner,
            super::StylesheetOwnerCspDisposition::from_blocked(
                style_enforced_violation.is_some() || script_enforced_violation.is_some(),
            ),
        );
        for violation in [
            style_report_only_violation,
            style_enforced_violation,
            script_report_only_violation,
            script_enforced_violation,
        ]
        .into_iter()
        .flatten()
        {
            self.queue_content_security_policy_violation_event_for_element_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
    }

    fn apply_link_style_element_csp_check<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        owner: DomHandle,
    ) {
        if !self.dom_host.is_connected(owner) {
            return;
        }
        let nonce = self
            .dom_host
            .node(owner)
            .and_then(Node::as_element)
            .and_then(|element| element.cryptographic_nonce())
            .map(str::to_owned);
        let node_id = NodeId::new(owner.index());
        let request_url =
            crate::stylesheet_blocking::stylesheet_link_disposition(&self.dom_host, node_id)
                .map(|disposition| disposition.url().clone())
                .or_else(|| {
                    crate::stylesheet_blocking::stylesheet_preload_link_request(
                        &self.dom_host,
                        node_id,
                    )
                    .map(|request| request.url().clone())
                });
        let Some(request_url) = request_url else {
            self.set_stylesheet_owner_csp_disposition(
                host_ptr,
                owner,
                super::StylesheetOwnerCspDisposition::Allowed,
            );
            return;
        };
        let (report_only_violation, enforced_violation) = self
            .style_element_request_csp_check(
                &request_url,
                ContentSecurityPolicyStyleElementRequest {
                    nonce: nonce.as_deref(),
                },
            )
            .into_violations();
        self.set_stylesheet_owner_csp_disposition(
            host_ptr,
            owner,
            super::StylesheetOwnerCspDisposition::from_blocked(enforced_violation.is_some()),
        );
        for violation in [report_only_violation, enforced_violation]
            .into_iter()
            .flatten()
        {
            self.queue_content_security_policy_violation_event_for_element_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
    }

    pub(crate) fn document_policy_container(&self) -> &DocumentPolicyContainer {
        &self.policy_container
    }

    pub(crate) fn set_bypass_content_security_policy(&mut self, bypass: bool) {
        self.bypass_content_security_policy = bypass;
    }

    pub(crate) fn bypass_content_security_policy(&self) -> bool {
        self.bypass_content_security_policy
    }

    pub(crate) fn set_document_content_security_policies(&mut self, policies: &[String]) {
        self.policy_container.document_content_security_policies = policies.to_vec();
    }

    pub(crate) fn document_content_security_policies(&self) -> &[String] {
        self.policy_container
            .document_content_security_policies
            .as_slice()
    }

    pub(crate) fn set_response_content_security_policies(&mut self, policies: &[String]) {
        self.policy_container.response_content_security_policies = policies.to_vec();
        self.policy_container.sandbox =
            DocumentSandboxPolicy::from_response_content_security_policies(policies);
    }

    pub(crate) fn response_content_security_policies(&self) -> &[String] {
        &self.policy_container.response_content_security_policies
    }

    pub(crate) fn set_response_content_security_report_only_policies(
        &mut self,
        policies: &[String],
    ) {
        self.policy_container
            .response_content_security_report_only_policies = policies.to_vec();
    }

    pub(crate) fn set_response_referrer_policy(&mut self, policy: Option<String>) {
        self.policy_container.referrer_policy = policy;
    }

    pub(crate) fn response_referrer_policy(&self) -> Option<&str> {
        self.policy_container.referrer_policy.as_deref()
    }

    pub(crate) fn set_cross_origin_embedder_policy(
        &mut self,
        policy: crate::cross_origin_isolation::CrossOriginEmbedderPolicy,
    ) {
        self.policy_container.cross_origin_embedder_policy = policy;
    }

    pub(crate) fn cross_origin_embedder_policy(
        &self,
    ) -> crate::cross_origin_isolation::CrossOriginEmbedderPolicy {
        self.policy_container.cross_origin_embedder_policy
    }

    pub(crate) fn set_document_isolation_policy(
        &mut self,
        policy: crate::cross_origin_isolation::DocumentIsolationPolicy,
    ) {
        self.policy_container.document_isolation_policy = policy;
    }

    pub(crate) fn document_isolation_policy(
        &self,
    ) -> crate::cross_origin_isolation::DocumentIsolationPolicy {
        self.policy_container.document_isolation_policy
    }

    pub(crate) fn set_cross_origin_isolated(&mut self, isolated: bool) {
        self.policy_container.cross_origin_isolated = isolated;
    }

    pub(crate) fn cross_origin_isolated(&self) -> bool {
        self.policy_container.cross_origin_isolated
    }

    pub(crate) fn ensure_credentialless_storage_nonce(
        &mut self,
        browser_context_runtime: &crate::runtime::RendererBrowserContextRuntime,
    ) -> moli_storage_key::OpaqueOriginNonce {
        if let Some(nonce) = self.policy_container.credentialless_storage_nonce {
            return nonce;
        }
        let nonce = browser_context_runtime.next_web_storage_opaque_context_nonce();
        self.policy_container.credentialless_storage_nonce = Some(nonce);
        nonce
    }

    pub(crate) fn document_sandbox_policy(&self) -> DocumentSandboxPolicy {
        self.policy_container.sandbox
    }

    pub(crate) fn response_content_security_report_only_policies(&self) -> &[String] {
        &self
            .policy_container
            .response_content_security_report_only_policies
    }

    pub(crate) fn set_content_security_reporting_endpoints(
        &mut self,
        endpoints: ContentSecurityPolicyReportingEndpoints,
    ) {
        self.policy_container.content_security_reporting_endpoints = endpoints;
    }

    pub(crate) fn content_security_reporting_endpoints(
        &self,
    ) -> &ContentSecurityPolicyReportingEndpoints {
        &self.policy_container.content_security_reporting_endpoints
    }

    #[cfg(test)]
    pub(crate) fn script_element_request_csp_violation(
        &self,
        request_url: &Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.script_element_request_csp_violation_with_nonce(request_url, None)
    }

    pub(crate) fn script_element_request_csp_violation_with_nonce(
        &self,
        request_url: &Url,
        nonce: Option<&str>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.script_element_request_csp_violation_with_request(
            request_url,
            ContentSecurityPolicyScriptElementRequest::parser_inserted_with_nonce(nonce),
        )
    }

    pub(crate) fn script_element_request_csp_violation_with_request(
        &self,
        request_url: &Url,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.script_element_request_csp_violation_with_redirect_status(
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            request,
        )
    }

    pub(crate) fn script_element_request_csp_violation_with_redirect_status(
        &self,
        request_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.document_url();
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_script_element_url_policy_violation_from_document_policies(
            policies,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Enforce,
            request,
        )
    }

    #[cfg(test)]
    pub(crate) fn script_element_request_csp_report_only_violation(
        &self,
        request_url: &Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.script_element_request_csp_report_only_violation_with_nonce(request_url, None)
    }

    pub(crate) fn script_element_request_csp_report_only_violation_with_nonce(
        &self,
        request_url: &Url,
        nonce: Option<&str>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.script_element_request_csp_report_only_violation_with_request(
            request_url,
            ContentSecurityPolicyScriptElementRequest::parser_inserted_with_nonce(nonce),
        )
    }

    pub(crate) fn script_element_request_csp_report_only_violation_with_request(
        &self,
        request_url: &Url,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.script_element_request_csp_report_only_violation_with_redirect_status(
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            request,
        )
    }

    pub(crate) fn script_element_request_csp_report_only_violation_with_redirect_status(
        &self,
        request_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.document_url();
        document_script_element_url_policy_violation(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Report,
            request,
        )
    }

    pub(crate) fn document_connect_csp_check_with_redirect_status(
        &self,
        request_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> DocumentContentSecurityPolicyCheck {
        let document_url = self.document_url();
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        let enforced_violation = document_connect_policy_violation_from_document_policies(
            policies,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Enforce,
        );
        let report_only_violation = document_connect_policy_violation(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
            self.document_url(),
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Report,
        );
        DocumentContentSecurityPolicyCheck {
            report_only_violation,
            enforced_violation,
        }
    }

    pub(crate) fn document_subresource_csp_check(
        &self,
        request_url: &Url,
        kind: DocumentSubresourceCspKind,
    ) -> DocumentContentSecurityPolicyCheck {
        let document_url = self.document_url();
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        let enforced_violation = document_url_policy_violation_from_document_policies(
            policies,
            document_url,
            request_url,
            kind.resource_kind(),
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
        );
        let report_only_violation = document_url_policy_violation(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
            document_url,
            request_url,
            kind.resource_kind(),
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Report,
        );
        DocumentContentSecurityPolicyCheck {
            report_only_violation,
            enforced_violation,
        }
    }

    pub(crate) fn inline_source_csp_check(
        &self,
        kind: ContentSecurityPolicyNonUrlKind,
        source: &str,
    ) -> DocumentContentSecurityPolicyCheck {
        self.inline_source_csp_check_for_child_document(
            Some(self.document_handle()),
            self.document_url(),
            &self.policy_container.response_content_security_policies,
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
            kind,
            source,
        )
    }

    pub(crate) fn inline_script_element_csp_check(
        &self,
        source: &str,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> DocumentContentSecurityPolicyCheck {
        self.inline_script_element_csp_check_for_child_document(
            Some(self.document_handle()),
            self.document_url(),
            &self.policy_container.response_content_security_policies,
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
            source,
            request,
        )
    }

    pub(crate) fn inline_style_element_csp_check(
        &self,
        source: &str,
        request: ContentSecurityPolicyStyleElementRequest<'_>,
    ) -> DocumentContentSecurityPolicyCheck {
        let document_url = self.document_url();
        let enforced_policies = self
            .document_content_security_policy_strings_for_optional_document(
                Some(self.document_handle()),
                &self.policy_container.response_content_security_policies,
                &self.policy_container.content_security_reporting_endpoints,
            );
        let enforced_violation =
            document_inline_style_element_policy_violation_from_document_policies(
                enforced_policies,
                document_url,
                source,
                request,
                ContentSecurityPolicyDisposition::Enforce,
            );
        let report_only_policies = document_response_content_security_policy_strings(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        let report_only_violation =
            document_inline_style_element_policy_violation_from_document_policies(
                report_only_policies,
                document_url,
                source,
                request,
                ContentSecurityPolicyDisposition::Report,
            );
        DocumentContentSecurityPolicyCheck {
            report_only_violation,
            enforced_violation,
        }
    }

    pub(crate) fn style_element_request_csp_check(
        &self,
        request_url: &Url,
        request: ContentSecurityPolicyStyleElementRequest<'_>,
    ) -> DocumentContentSecurityPolicyCheck {
        let document_url = self.document_url();
        let enforced_policies = self
            .document_content_security_policy_strings_for_optional_document(
                Some(self.document_handle()),
                &self.policy_container.response_content_security_policies,
                &self.policy_container.content_security_reporting_endpoints,
            );
        let enforced_violation = document_style_element_url_policy_violation_from_document_policies(
            enforced_policies,
            document_url,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            request,
        );
        let report_only_policies = document_response_content_security_policy_strings(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        let report_only_violation =
            document_style_element_url_policy_violation_from_document_policies(
                report_only_policies,
                document_url,
                request_url,
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Report,
                request,
            );
        DocumentContentSecurityPolicyCheck {
            report_only_violation,
            enforced_violation,
        }
    }

    pub(crate) fn response_style_element_request_csp_check(
        &self,
        request_url: &Url,
        request: ContentSecurityPolicyStyleElementRequest<'_>,
    ) -> DocumentContentSecurityPolicyCheck {
        let document_url = self.document_url();
        let enforced_policies = document_response_content_security_policy_strings(
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        let enforced_violation = document_style_element_url_policy_violation_from_document_policies(
            enforced_policies,
            document_url,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            request,
        );
        let report_only_policies = document_response_content_security_policy_strings(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        let report_only_violation =
            document_style_element_url_policy_violation_from_document_policies(
                report_only_policies,
                document_url,
                request_url,
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Report,
                request,
            );
        DocumentContentSecurityPolicyCheck {
            report_only_violation,
            enforced_violation,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn inline_script_element_csp_check_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        source: &str,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> DocumentContentSecurityPolicyCheck {
        let enforced_policies = self
            .document_content_security_policy_strings_for_optional_document(
                document_handle,
                response_policies,
                response_reporting_endpoints,
            );
        let enforced_violation =
            document_inline_script_element_policy_violation_from_document_policies(
                enforced_policies,
                document_url,
                source,
                request,
                ContentSecurityPolicyDisposition::Enforce,
            );
        let report_only_policies = document_response_content_security_policy_strings(
            response_report_only_policies,
            response_reporting_endpoints,
        );
        let report_only_violation =
            document_inline_script_element_policy_violation_from_document_policies(
                report_only_policies,
                document_url,
                source,
                request,
                ContentSecurityPolicyDisposition::Report,
            );
        DocumentContentSecurityPolicyCheck {
            report_only_violation,
            enforced_violation,
        }
    }

    pub(crate) fn inline_source_csp_check_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        kind: ContentSecurityPolicyNonUrlKind,
        source: &str,
    ) -> DocumentContentSecurityPolicyCheck {
        let enforced_policies = self
            .document_content_security_policy_strings_for_optional_document(
                document_handle,
                response_policies,
                response_reporting_endpoints,
            );
        let enforced_violation = document_inline_source_policy_violation_from_document_policies(
            enforced_policies,
            document_url,
            kind,
            source,
            ContentSecurityPolicyDisposition::Enforce,
        );
        let report_only_policies = document_response_content_security_policy_strings(
            response_report_only_policies,
            response_reporting_endpoints,
        );
        let report_only_violation = document_inline_source_policy_violation_from_document_policies(
            report_only_policies,
            document_url,
            kind,
            source,
            ContentSecurityPolicyDisposition::Report,
        );
        DocumentContentSecurityPolicyCheck {
            report_only_violation,
            enforced_violation,
        }
    }

    pub(crate) fn document_frame_csp_violation(
        &self,
        request_url: &Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.document_url();
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_frame_policy_violation_from_document_policies(
            policies,
            document_url,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn document_frame_csp_report_only_violation(
        &self,
        request_url: &Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        document_frame_policy_violation(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
            self.document_url(),
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Report,
        )
    }

    pub(crate) fn script_element_request_csp_violation_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        request_url: &Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.script_element_request_csp_violation_for_child_document_with_redirect_status(
            document_handle,
            document_url,
            response_policies,
            response_reporting_endpoints,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            None,
        )
    }

    pub(crate) fn script_element_request_csp_violation_for_child_document_with_redirect_status(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        request_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
        nonce: Option<&str>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_script_element_url_policy_violation_from_document_policies(
            policies,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Enforce,
            ContentSecurityPolicyScriptElementRequest::parser_inserted_with_nonce(nonce),
        )
    }

    pub(crate) fn script_element_request_csp_report_only_violation_for_child_document(
        &self,
        document_url: &Url,
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        request_url: &Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.script_element_request_csp_report_only_violation_for_child_document_with_redirect_status(
            document_url,
            response_report_only_policies,
            response_reporting_endpoints,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            None,
        )
    }

    pub(crate) fn script_element_request_csp_report_only_violation_for_child_document_with_redirect_status(
        &self,
        document_url: &Url,
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        request_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
        nonce: Option<&str>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        document_script_element_url_policy_violation(
            response_report_only_policies,
            response_reporting_endpoints,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Report,
            ContentSecurityPolicyScriptElementRequest::parser_inserted_with_nonce(nonce),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn document_connect_csp_check_for_child_document_with_redirect_status(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        request_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> DocumentContentSecurityPolicyCheck {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        let enforced_violation = document_connect_policy_violation_from_document_policies(
            policies,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Enforce,
        );
        let report_only_violation = document_connect_policy_violation(
            response_report_only_policies,
            response_reporting_endpoints,
            document_url,
            request_url,
            redirect_status,
            ContentSecurityPolicyDisposition::Report,
        );
        DocumentContentSecurityPolicyCheck {
            report_only_violation,
            enforced_violation,
        }
    }

    pub(crate) fn document_frame_csp_violation_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        request_url: &Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_frame_policy_violation_from_document_policies(
            policies,
            document_url,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn document_frame_csp_report_only_violation_for_child_document(
        &self,
        document_url: &Url,
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        request_url: &Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        document_frame_policy_violation(
            response_report_only_policies,
            response_reporting_endpoints,
            document_url,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Report,
        )
    }

    pub(crate) fn inline_script_csp_violation_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_non_url_policy_violation_from_document_policies(
            policies,
            document_url,
            ContentSecurityPolicyNonUrlKind::DocumentInlineScript,
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn inline_script_csp_report_only_violation_for_child_document(
        &self,
        document_url: &Url,
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = document_response_content_security_policy_strings(
            response_report_only_policies,
            response_reporting_endpoints,
        );
        document_non_url_policy_violation_from_document_policies(
            policies,
            document_url,
            ContentSecurityPolicyNonUrlKind::DocumentInlineScript,
            ContentSecurityPolicyDisposition::Report,
        )
    }

    pub(crate) fn wasm_eval_csp_violation(&self) -> Option<DocumentContentSecurityPolicyViolation> {
        self.non_url_csp_violation(
            ContentSecurityPolicyNonUrlKind::WasmEval,
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn eval_csp_violation(
        &self,
        allow_trusted_types_eval: bool,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.non_url_csp_violation(
            eval_csp_kind(allow_trusted_types_eval),
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn eval_csp_report_only_violation(
        &self,
        allow_trusted_types_eval: bool,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.non_url_csp_report_only_violation(
            eval_csp_kind(allow_trusted_types_eval),
            ContentSecurityPolicyDisposition::Report,
        )
    }

    fn non_url_csp_violation(
        &self,
        kind: ContentSecurityPolicyNonUrlKind,
        disposition: ContentSecurityPolicyDisposition,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.document_url();
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_non_url_policy_violation_from_document_policies(
            policies,
            document_url,
            kind,
            disposition,
        )
    }

    fn non_url_csp_report_only_violation(
        &self,
        kind: ContentSecurityPolicyNonUrlKind,
        disposition: ContentSecurityPolicyDisposition,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.document_url();
        let policies = document_response_content_security_policy_strings(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_non_url_policy_violation_from_document_policies(
            policies,
            document_url,
            kind,
            disposition,
        )
    }

    pub(crate) fn wasm_eval_csp_report_only_violation(
        &self,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.non_url_csp_report_only_violation(
            ContentSecurityPolicyNonUrlKind::WasmEval,
            ContentSecurityPolicyDisposition::Report,
        )
    }

    pub(crate) fn trusted_types_sink_csp_violation(
        &self,
        sink: &str,
        sample: &str,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.document_url();
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_trusted_types_sink_policy_violation_from_document_policies(
            policies,
            document_url,
            sink,
            sample,
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn trusted_types_sink_csp_report_only_violation(
        &self,
        sink: &str,
        sample: &str,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.document_url();
        let policies = document_response_content_security_policy_strings(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_trusted_types_sink_policy_violation_from_document_policies(
            policies,
            document_url,
            sink,
            sample,
            ContentSecurityPolicyDisposition::Report,
        )
    }

    pub(crate) fn trusted_types_for_script_requirements(
        &self,
    ) -> TrustedTypesForScriptRequirements {
        let enforced = self.requires_trusted_types_for_script();
        let report_only_policies = document_response_content_security_policy_strings(
            &self
                .policy_container
                .response_content_security_report_only_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        TrustedTypesForScriptRequirements::new(
            enforced,
            document_policies_require_trusted_types_for_script(&report_only_policies),
        )
    }

    pub(crate) fn requires_trusted_types_for_script(&self) -> bool {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_policies_require_trusted_types_for_script(&policies)
    }

    pub(crate) fn allows_trusted_types_eval(&self) -> bool {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_policies_allow_trusted_types_eval(&policies)
    }

    pub(crate) fn allows_trusted_type_policy_name(&self, policy_name: &str) -> bool {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            Some(self.document_handle()),
            &self.policy_container.response_content_security_policies,
            &self.policy_container.content_security_reporting_endpoints,
        );
        document_policies_allow_trusted_type_policy_name(policies, policy_name)
    }

    pub(crate) fn wasm_eval_csp_violation_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_non_url_policy_violation_from_document_policies(
            policies,
            document_url,
            ContentSecurityPolicyNonUrlKind::WasmEval,
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn eval_csp_violation_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        allow_trusted_types_eval: bool,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_non_url_policy_violation_from_document_policies(
            policies,
            document_url,
            eval_csp_kind(allow_trusted_types_eval),
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn trusted_types_sink_csp_violation_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        document_url: &Url,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        sink: &str,
        sample: &str,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_trusted_types_sink_policy_violation_from_document_policies(
            policies,
            document_url,
            sink,
            sample,
            ContentSecurityPolicyDisposition::Enforce,
        )
    }

    pub(crate) fn requires_trusted_types_for_script_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    ) -> bool {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_policies_require_trusted_types_for_script(&policies)
    }

    pub(crate) fn allows_trusted_types_eval_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    ) -> bool {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_policies_allow_trusted_types_eval(&policies)
    }

    pub(crate) fn allows_trusted_type_policy_name_for_child_document(
        &self,
        document_handle: Option<DomHandle>,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        policy_name: &str,
    ) -> bool {
        let policies = self.document_content_security_policy_strings_for_optional_document(
            document_handle,
            response_policies,
            response_reporting_endpoints,
        );
        document_policies_allow_trusted_type_policy_name(policies, policy_name)
    }

    pub(crate) fn wasm_eval_csp_report_only_violation_for_child_document(
        &self,
        document_url: &Url,
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = document_response_content_security_policy_strings(
            response_report_only_policies,
            response_reporting_endpoints,
        );
        document_non_url_policy_violation_from_document_policies(
            policies,
            document_url,
            ContentSecurityPolicyNonUrlKind::WasmEval,
            ContentSecurityPolicyDisposition::Report,
        )
    }

    pub(crate) fn eval_csp_report_only_violation_for_child_document(
        &self,
        document_url: &Url,
        response_report_only_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
        allow_trusted_types_eval: bool,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let policies = document_response_content_security_policy_strings(
            response_report_only_policies,
            response_reporting_endpoints,
        );
        document_non_url_policy_violation_from_document_policies(
            policies,
            document_url,
            eval_csp_kind(allow_trusted_types_eval),
            ContentSecurityPolicyDisposition::Report,
        )
    }

    pub(crate) fn queue_content_security_policy_violation_event_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        if let Err(error) =
            self.queue_content_security_policy_violation_event(scope, host_ptr, violation)
        {
            tracing::error!(
                blocked_uri = violation.blocked_uri.as_str(),
                message = error.to_string().as_str(),
                "securitypolicyviolation queueing failed"
            );
        }
    }

    pub(crate) fn queue_content_security_policy_violation_event_for_element_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        target: DomHandle,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        if let Err(error) = self.queue_content_security_policy_violation_event_for_target(
            scope,
            host_ptr,
            Some(target),
            violation,
            true,
        ) {
            tracing::error!(
                blocked_uri = violation.blocked_uri.as_str(),
                message = error.to_string().as_str(),
                "element securitypolicyviolation queueing failed"
            );
        }
    }

    pub(crate) fn queue_content_security_policy_violation_event_without_report_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        if let Err(error) = self.queue_content_security_policy_violation_event_for_target(
            scope, host_ptr, None, violation, false,
        ) {
            tracing::error!(
                blocked_uri = violation.blocked_uri.as_str(),
                message = error.to_string().as_str(),
                "securitypolicyviolation event-only queueing failed"
            );
        }
    }

    pub(crate) fn queue_content_security_policy_violation_event<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        violation: &DocumentContentSecurityPolicyViolation,
    ) -> anyhow::Result<()> {
        self.queue_content_security_policy_violation_event_for_target(
            scope, host_ptr, None, violation, true,
        )
    }

    fn queue_content_security_policy_violation_event_for_target<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        target: Option<DomHandle>,
        violation: &DocumentContentSecurityPolicyViolation,
        send_report: bool,
    ) -> anyhow::Result<()> {
        self.record_content_security_policy_inspector_issue(target, violation);
        let host = unsafe { &mut *host_ptr };
        let document_owner = host
            .current_main_document_task_owner()
            .ok_or_else(|| anyhow::anyhow!("main CSP violation document owner is unavailable"))?;
        if send_report {
            let fields = ContentSecurityPolicyViolationEventFields::from(violation);
            crate::network_host::send_content_security_policy_reports_for_window(
                scope,
                host,
                document_owner,
                None,
                &fields,
                &violation.report_uri_endpoints,
                &violation.report_to_endpoints,
            );
        }
        let event_task = match target {
            Some(target) => {
                crate::page_task_queue::ContentSecurityPolicyViolationEventTask::for_element(
                    document_owner,
                    target,
                    violation.clone(),
                )
            }
            None => crate::page_task_queue::ContentSecurityPolicyViolationEventTask::new(
                document_owner,
                violation.clone(),
            ),
        };
        let work = PostParseLifecycleWork::DispatchContentSecurityPolicyViolation(event_task);
        if self.has_active_parser_write_insertion_point() {
            self.enqueue_parser_boundary_lifecycle_work(work);
        } else if !self.dom_content_loaded_dispatched() {
            self.enqueue_parser_owned_pre_domcontentloaded_page_owned_work(
                PostParsePageOwnedWork::lifecycle_work(work),
            );
        } else {
            self.script_lifecycle
                .scripts_mut()
                .enqueue_post_parse_lifecycle_work(work);
        }
        Ok(())
    }

    pub(crate) fn dispatch_content_security_policy_violation_event_page_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        task: &crate::page_task_queue::ContentSecurityPolicyViolationEventTask,
    ) -> anyhow::Result<()> {
        if unsafe { &*host_ptr }.current_main_document_task_owner() != Some(task.owner()) {
            return Ok(());
        }
        let violation = task.violation();
        let event_target =
            EventTargetHandle::Node(task.target().unwrap_or_else(|| self.document_handle()));
        let event_target_value =
            event_target_value(scope, host_ptr, event_target).map_err(anyhow::Error::msg)?;
        let event = create_content_security_policy_violation_event(
            scope,
            event_target_value,
            event_target_value,
            violation,
        )
        .map_err(anyhow::Error::msg)?;
        self.dispatch_public_event_best_effort(
            scope,
            host_ptr,
            event_target,
            event,
            "securitypolicyviolation",
        )
        .map_err(anyhow::Error::msg)?;
        Ok(())
    }

    fn document_content_security_policy_strings_for_optional_document(
        &self,
        document_handle: Option<DomHandle>,
        response_policies: &[String],
        response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    ) -> Vec<DocumentContentSecurityPolicyString> {
        let mut policies = document_response_content_security_policy_strings(
            response_policies,
            response_reporting_endpoints,
        );
        if let Some(handle) = document_handle {
            policies.extend(
                self.meta_content_security_policy_strings_for_document(handle)
                    .into_iter()
                    .map(|policy| DocumentContentSecurityPolicyString {
                        policy,
                        report_uri_enabled: false,
                        reporting_endpoints: self
                            .policy_container
                            .content_security_reporting_endpoints
                            .clone(),
                    }),
            );
        }
        policies
    }

    fn meta_content_security_policy_strings_for_document(
        &self,
        document_handle: DomHandle,
    ) -> Vec<String> {
        let candidates = self
            .dom_host
            .html_elements_by_local_name_in_document_tree_order(document_handle, "meta")
            .into_iter();
        for handle in candidates {
            self.process_meta_content_security_policy_handle(document_handle, handle);
        }
        self.delivered_meta_content_security_policies
            .borrow()
            .get(&document_handle)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn process_parser_meta_content_security_policy(&self, handle: DomHandle) {
        self.process_meta_content_security_policy_handle(self.document_handle(), handle);
    }

    fn process_meta_content_security_policy_handle(
        &self,
        document_handle: DomHandle,
        handle: DomHandle,
    ) {
        if self.dom_host.owner_document_handle(handle) != Some(document_handle) {
            return;
        }
        let policy = {
            let Some(element) = self
                .dom_host
                .node(handle)
                .and_then(|node| node.as_element())
            else {
                return;
            };
            if element.namespace() != "http://www.w3.org/1999/xhtml"
                || !element.local_name().eq_ignore_ascii_case("meta")
                || !element
                    .attribute("http-equiv")
                    .is_some_and(|value| value.eq_ignore_ascii_case("content-security-policy"))
            {
                return;
            }
            element
                .attribute("content")
                .map(str::trim)
                .filter(|content| !content.is_empty())
                .map(str::to_owned)
        };
        if !self
            .processed_meta_content_security_policy_handles
            .borrow_mut()
            .insert((document_handle, handle))
            || self.bypass_content_security_policy
        {
            return;
        }
        if let Some(policy) = policy {
            self.delivered_meta_content_security_policies
                .borrow_mut()
                .entry(document_handle)
                .or_default()
                .push(policy);
        }
    }
}

fn document_connect_policy_violation(
    policies: &[String],
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    document_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    document_url_policy_violation(
        policies,
        reporting_endpoints,
        document_url,
        request_url,
        ContentSecurityPolicyResourceKind::DocumentConnect,
        redirect_status,
        disposition,
    )
}

fn document_frame_policy_violation(
    policies: &[String],
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    document_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    document_url_policy_violation(
        policies,
        reporting_endpoints,
        document_url,
        request_url,
        ContentSecurityPolicyResourceKind::DocumentFrame,
        redirect_status,
        disposition,
    )
}

fn document_url_policy_violation(
    policies: &[String],
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    document_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
        policies,
        document_url,
        request_url,
        kind,
        redirect_status,
        disposition,
        reporting_endpoints,
    )
}

fn document_script_element_url_policy_violation(
    policies: &[String],
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    document_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
) -> Option<DocumentContentSecurityPolicyViolation> {
    content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
        policies,
        document_url,
        request_url,
        redirect_status,
        disposition,
        reporting_endpoints,
        request,
    )
}

fn document_style_element_url_policy_violation(
    policies: &[String],
    reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
    document_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    request: ContentSecurityPolicyStyleElementRequest<'_>,
) -> Option<DocumentContentSecurityPolicyViolation> {
    content_security_policy_style_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
        policies,
        document_url,
        request_url,
        redirect_status,
        disposition,
        reporting_endpoints,
        request,
    )
}

#[derive(Debug, Clone)]
struct DocumentContentSecurityPolicyString {
    policy: String,
    report_uri_enabled: bool,
    reporting_endpoints: ContentSecurityPolicyReportingEndpoints,
}

fn document_response_content_security_policy_strings(
    response_policies: &[String],
    response_reporting_endpoints: &ContentSecurityPolicyReportingEndpoints,
) -> Vec<DocumentContentSecurityPolicyString> {
    response_policies
        .iter()
        .map(|policy| DocumentContentSecurityPolicyString {
            policy: policy.to_owned(),
            report_uri_enabled: true,
            reporting_endpoints: response_reporting_endpoints.clone(),
        })
        .collect()
}

fn document_connect_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    document_url_policy_violation_from_document_policies(
        policies,
        document_url,
        request_url,
        ContentSecurityPolicyResourceKind::DocumentConnect,
        redirect_status,
        disposition,
    )
}

fn document_frame_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    document_url_policy_violation_from_document_policies(
        policies,
        document_url,
        request_url,
        ContentSecurityPolicyResourceKind::DocumentFrame,
        redirect_status,
        disposition,
    )
}

fn document_url_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    policies.into_iter().find_map(|policy| {
        let single_policy = [policy.policy.clone()];
        let mut violation = document_url_policy_violation(
            &single_policy,
            &policy.reporting_endpoints,
            document_url,
            request_url,
            kind,
            redirect_status,
            disposition,
        )?;
        apply_document_policy_reporting_flags(&mut violation, &policy);
        Some(violation)
    })
}

fn document_script_element_url_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
) -> Option<DocumentContentSecurityPolicyViolation> {
    policies.into_iter().find_map(|policy| {
        let single_policy = [policy.policy.clone()];
        let mut violation = document_script_element_url_policy_violation(
            &single_policy,
            &policy.reporting_endpoints,
            document_url,
            request_url,
            redirect_status,
            disposition,
            request,
        )?;
        apply_document_policy_reporting_flags(&mut violation, &policy);
        Some(violation)
    })
}

fn document_style_element_url_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    request_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
    request: ContentSecurityPolicyStyleElementRequest<'_>,
) -> Option<DocumentContentSecurityPolicyViolation> {
    policies.into_iter().find_map(|policy| {
        let single_policy = [policy.policy.clone()];
        let mut violation = document_style_element_url_policy_violation(
            &single_policy,
            &policy.reporting_endpoints,
            document_url,
            request_url,
            redirect_status,
            disposition,
            request,
        )?;
        apply_document_policy_reporting_flags(&mut violation, &policy);
        Some(violation)
    })
}

fn document_non_url_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    kind: ContentSecurityPolicyNonUrlKind,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    policies.into_iter().find_map(|policy| {
        let mut violation =
            content_security_policy_non_url_violation_with_disposition_and_reporting_endpoints(
                &policy.policy,
                document_url,
                kind,
                disposition,
                &policy.reporting_endpoints,
            )?;
        apply_document_policy_reporting_flags(&mut violation, &policy);
        Some(violation)
    })
}

fn document_inline_source_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    kind: ContentSecurityPolicyNonUrlKind,
    source: &str,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    policies.into_iter().find_map(|policy| {
        let mut violation = content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
            &policy.policy,
            document_url,
            kind,
            source,
            disposition,
            &policy.reporting_endpoints,
        )?;
        apply_document_policy_reporting_flags(&mut violation, &policy);
        Some(violation)
    })
}

fn document_inline_script_element_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    source: &str,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    policies.into_iter().find_map(|policy| {
        let mut violation = content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
            &policy.policy,
            document_url,
            source,
            request,
            disposition,
            &policy.reporting_endpoints,
        )?;
        apply_document_policy_reporting_flags(&mut violation, &policy);
        Some(violation)
    })
}

fn document_inline_style_element_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    source: &str,
    request: ContentSecurityPolicyStyleElementRequest<'_>,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    policies.into_iter().find_map(|policy| {
        let mut violation = content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
            &policy.policy,
            document_url,
            source,
            request,
            disposition,
            &policy.reporting_endpoints,
        )?;
        apply_document_policy_reporting_flags(&mut violation, &policy);
        Some(violation)
    })
}

fn document_trusted_types_sink_policy_violation_from_document_policies(
    policies: Vec<DocumentContentSecurityPolicyString>,
    document_url: &Url,
    sink: &str,
    sample: &str,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<DocumentContentSecurityPolicyViolation> {
    policies.into_iter().find_map(|policy| {
        let single_policy = [policy.policy.clone()];
        let mut violation =
            content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints(
                &single_policy,
                document_url,
                sink,
                sample,
                disposition,
                &policy.reporting_endpoints,
            )?;
        apply_document_policy_reporting_flags(&mut violation, &policy);
        Some(violation)
    })
}

fn document_policies_allow_trusted_type_policy_name(
    policies: Vec<DocumentContentSecurityPolicyString>,
    policy_name: &str,
) -> bool {
    policies.into_iter().all(|policy| {
        content_security_policy_allows_trusted_type_policy_name(&[policy.policy], policy_name)
    })
}

fn document_policies_require_trusted_types_for_script(
    policies: &[DocumentContentSecurityPolicyString],
) -> bool {
    policies.iter().any(|policy| {
        content_security_policy_requires_trusted_types_for_script(std::slice::from_ref(
            &policy.policy,
        ))
    })
}

fn eval_csp_kind(allow_trusted_types_eval: bool) -> ContentSecurityPolicyNonUrlKind {
    if allow_trusted_types_eval {
        ContentSecurityPolicyNonUrlKind::TrustedTypesEval
    } else {
        ContentSecurityPolicyNonUrlKind::Eval
    }
}

fn document_policies_allow_trusted_types_eval(
    policies: &[DocumentContentSecurityPolicyString],
) -> bool {
    let policies = policies
        .iter()
        .map(|policy| policy.policy.clone())
        .collect::<Vec<_>>();
    content_security_policy_allows_trusted_types_eval(&policies)
}

fn apply_document_policy_reporting_flags(
    violation: &mut DocumentContentSecurityPolicyViolation,
    policy: &DocumentContentSecurityPolicyString,
) {
    if !policy.report_uri_enabled {
        violation.report_uri_endpoints.clear();
    }
}

pub(crate) fn document_content_security_policy_error_message(
    violation: &DocumentContentSecurityPolicyViolation,
    operation: &'static str,
) -> String {
    format!(
        "{operation}: blocked by Content Security Policy for `{}`.",
        violation.blocked_uri
    )
}

pub(crate) fn create_content_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    current_target: v8::Local<'s, v8::Value>,
    target: v8::Local<'s, v8::Value>,
    violation: &DocumentContentSecurityPolicyViolation,
) -> anyhow::Result<v8::Local<'s, v8::Object>> {
    let event = create_security_policy_violation_event(
        scope,
        &ContentSecurityPolicyViolationEventFields::from(violation),
    )
    .ok_or_else(|| anyhow::anyhow!("failed to create CSP violation event"))?;
    let _ = event.set(scope, v8str(scope, "target").into(), target);
    let _ = event.set(scope, v8str(scope, "srcElement").into(), target);
    let _ = event.set(scope, v8str(scope, "currentTarget").into(), current_target);
    Ok(event)
}

pub(crate) fn response_content_security_policies_from_headers(
    headers: &[(String, String)],
) -> Vec<String> {
    content_security_policy_headers(headers)
}

pub(crate) fn response_content_security_report_only_policies_from_headers(
    headers: &[(String, String)],
) -> Vec<String> {
    content_security_policy_report_only_headers(headers)
}

impl<'a> From<&'a DocumentContentSecurityPolicyViolation>
    for ContentSecurityPolicyViolationEventFields<'a>
{
    fn from(violation: &'a DocumentContentSecurityPolicyViolation) -> Self {
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
            line_number: violation.line_number,
            column_number: violation.column_number,
            status_code: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::HtmlParser;

    fn runtime_for_html(html: &str) -> DocumentRuntime {
        let document_url = Url::parse("https://example.test/page").expect("document url");
        let document = HtmlParser.parse(document_url, html.to_owned());
        DocumentRuntime::from_document(document)
    }

    fn runtime_with_response_csp(policy: &str) -> DocumentRuntime {
        let mut runtime = runtime_for_html("<!doctype html>");
        runtime.set_response_content_security_policies(&[policy.to_owned()]);
        runtime
    }

    fn runtime_with_response_csp_report_only(policy: &str) -> DocumentRuntime {
        let mut runtime = runtime_for_html("<!doctype html>");
        runtime.set_response_content_security_report_only_policies(&[policy.to_owned()]);
        runtime
    }

    #[test]
    fn trusted_types_report_only_requirements_ignore_enforced_meta_policies() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy"
                  content="require-trusted-types-for 'script'">"#,
        );

        let requirements = runtime.trusted_types_for_script_requirements();
        assert!(requirements.requires_conversion());
        assert!(requirements.is_enforced());
        assert!(
            runtime
                .trusted_types_sink_csp_report_only_violation("Element innerHTML", "plain")
                .is_none()
        );
    }

    #[test]
    fn trusted_types_report_only_response_requires_conversion_without_enforcement() {
        let runtime = runtime_with_response_csp_report_only(
            "require-trusted-types-for 'script'; report-to csp",
        );

        let requirements = runtime.trusted_types_for_script_requirements();
        assert!(requirements.requires_conversion());
        assert!(!requirements.is_enforced());
        let violation = runtime
            .trusted_types_sink_csp_report_only_violation("Element innerHTML", "plain")
            .expect("report-only Trusted Types sink should produce a violation");
        assert_eq!(
            violation.disposition,
            ContentSecurityPolicyDisposition::Report
        );
    }

    #[test]
    fn response_csp_sandbox_updates_current_document_sandbox_policy_record() {
        let mut runtime = runtime_for_html("<!doctype html>");
        assert_eq!(
            runtime.document_sandbox_policy(),
            DocumentSandboxPolicy::default()
        );

        runtime.set_response_content_security_policies(&[String::from("sandbox")]);
        assert_eq!(
            runtime.document_sandbox_policy(),
            DocumentSandboxPolicy {
                forces_opaque_origin: true,
                allows_scripts: false,
                allows_popups_to_escape: false,
                sandboxes_document_domain: true,
            }
        );

        runtime.set_response_content_security_policies(&[String::from(
            "sandbox allow-scripts allow-same-origin allow-popups-to-escape-sandbox",
        )]);
        assert_eq!(
            runtime.document_sandbox_policy(),
            DocumentSandboxPolicy {
                forces_opaque_origin: false,
                allows_scripts: true,
                allows_popups_to_escape: true,
                sandboxes_document_domain: true,
            }
        );

        runtime.set_response_content_security_policies(&[
            String::from("sandbox allow-scripts allow-popups-to-escape-sandbox"),
            String::from("sandbox allow-same-origin"),
        ]);
        assert_eq!(
            runtime.document_sandbox_policy(),
            DocumentSandboxPolicy {
                forces_opaque_origin: true,
                allows_scripts: false,
                allows_popups_to_escape: false,
                sandboxes_document_domain: true,
            }
        );

        runtime.set_response_content_security_policies(&[String::from("script-src 'self'")]);
        assert_eq!(
            runtime.document_sandbox_policy(),
            DocumentSandboxPolicy::default()
        );
    }

    fn reporting_endpoints() -> ContentSecurityPolicyReportingEndpoints {
        crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
            &[(
                "Reporting-Endpoints".to_owned(),
                "csp=\"/reports/csp\"".to_owned(),
            )],
            &Url::parse("https://example.test/page").unwrap(),
        )
    }

    #[test]
    fn csp_script_src_self_blocks_cross_origin_script_element_request() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src 'self' 'unsafe-inline'">
            "#,
        );
        let blocked_url = Url::parse("https://cdn.example.test/module.wasm").unwrap();
        let violation = runtime
            .script_element_request_csp_violation(&blocked_url)
            .expect("cross-origin request should violate script-src self");

        assert_eq!(violation.effective_directive, "script-src-elem");
        assert_eq!(
            violation.original_policy,
            "script-src 'self' 'unsafe-inline'"
        );
        assert_eq!(
            violation.disposition,
            ContentSecurityPolicyDisposition::Enforce
        );
    }

    #[test]
    fn delivered_meta_csp_policy_ignores_later_content_attribute_changes() {
        let mut runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="img-src 'none'">
            "#,
        );
        let request_url = Url::parse("https://example.test/image.png").expect("image URL");
        let initial_check =
            runtime.document_subresource_csp_check(&request_url, DocumentSubresourceCspKind::Image);
        let initial = initial_check
            .enforced_violation()
            .expect("initial meta CSP should block image");
        assert_eq!(initial.original_policy, "img-src 'none'");

        let meta = runtime
            .dom_host()
            .html_elements_by_local_name_in_document_tree_order(runtime.document_handle(), "meta")
            .into_iter()
            .next()
            .expect("meta element");
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(meta, "content", "img-src *")
        );

        let after_mutation_check =
            runtime.document_subresource_csp_check(&request_url, DocumentSubresourceCspKind::Image);
        let after_mutation = after_mutation_check
            .enforced_violation()
            .expect("delivered meta CSP should remain enforced");
        assert_eq!(after_mutation.original_policy, "img-src 'none'");
    }

    #[test]
    fn child_document_meta_csp_does_not_leak_into_main_document() {
        let mut runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src 'unsafe-eval'">
            "#,
        );
        let child_document = runtime.dom_host_mut().create_detached_html_document();
        let child_meta = runtime.dom_host_mut().create_element("meta");
        assert_eq!(
            runtime
                .dom_host_mut()
                .adopt_node(child_document, child_meta),
            Some(child_meta)
        );
        assert!(runtime.dom_host_mut().set_attribute(
            child_meta,
            "http-equiv",
            "Content-Security-Policy"
        ));
        assert!(runtime.dom_host_mut().set_attribute(
            child_meta,
            "content",
            "require-trusted-types-for 'script'"
        ));
        assert!(
            runtime
                .dom_host_mut()
                .append_child(child_document, child_meta)
        );
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();

        assert!(
            runtime.requires_trusted_types_for_script_for_child_document(
                Some(child_document),
                &[],
                &reporting_endpoints,
            )
        );
        assert!(
            !runtime.requires_trusted_types_for_script(),
            "a child document's meta policy must not affect the parent document"
        );
        assert!(
            runtime.requires_trusted_types_for_script_for_child_document(
                Some(child_document),
                &[],
                &reporting_endpoints,
            ),
            "checking the parent must not discard the child document's delivered policy"
        );
    }

    #[test]
    fn document_open_does_not_reuse_replaced_document_meta_csp() {
        let mut runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy"
                  content="require-trusted-types-for 'script'">
            "#,
        );
        assert!(runtime.requires_trusted_types_for_script());

        runtime.open_document();

        assert!(
            !runtime.requires_trusted_types_for_script(),
            "the replacement document must not inherit delivered meta policies"
        );
    }

    #[test]
    fn csp_bypass_does_not_clear_policy_installed_in_current_document() {
        let mut runtime = runtime_with_response_csp("img-src 'none'");
        let request_url = Url::parse("https://example.test/image.png").expect("image URL");

        runtime.set_bypass_content_security_policy(true);

        assert!(
            runtime
                .document_subresource_csp_check(&request_url, DocumentSubresourceCspKind::Image,)
                .enforced_violation()
                .is_some(),
            "Page.setBypassCSP affects future document policy installation, not the current document's installed CSP"
        );
    }

    #[test]
    fn meta_csp_seen_while_bypass_is_enabled_is_not_installed_later() {
        let mut runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="img-src 'none'">
            "#,
        );
        let request_url = Url::parse("https://example.test/image.png").expect("image URL");

        runtime.set_bypass_content_security_policy(true);
        assert!(
            runtime
                .document_subresource_csp_check(&request_url, DocumentSubresourceCspKind::Image,)
                .enforced_violation()
                .is_none()
        );

        runtime.set_bypass_content_security_policy(false);
        assert!(
            runtime
                .document_subresource_csp_check(&request_url, DocumentSubresourceCspKind::Image,)
                .enforced_violation()
                .is_none(),
            "a meta policy ignored at delivery time must not be installed retroactively"
        );
    }

    #[test]
    fn document_subresource_csp_check_keeps_report_and_enforce_results_together() {
        let mut runtime = runtime_with_response_csp("img-src 'none'; media-src 'none'");
        runtime.set_response_content_security_report_only_policies(&[String::from(
            "img-src 'none'; media-src 'none'",
        )]);
        let request_url = Url::parse("https://example.test/resource").expect("resource URL");

        for (kind, directive) in [
            (DocumentSubresourceCspKind::Image, "img-src"),
            (DocumentSubresourceCspKind::Media, "media-src"),
        ] {
            let check = runtime.document_subresource_csp_check(&request_url, kind);
            let report_only = check
                .report_only_violation()
                .expect("report-only policy should produce a violation");
            let enforced = check
                .enforced_violation()
                .expect("enforced policy should produce a violation");

            assert_eq!(report_only.effective_directive, directive);
            assert_eq!(
                report_only.disposition,
                ContentSecurityPolicyDisposition::Report
            );
            assert_eq!(enforced.effective_directive, directive);
            assert_eq!(
                enforced.disposition,
                ContentSecurityPolicyDisposition::Enforce
            );
        }
    }

    #[test]
    fn csp_script_src_self_allows_same_origin_script_element_request() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src 'self' 'unsafe-inline'">
            "#,
        );
        let allowed_url = Url::parse("https://example.test/module.wasm").unwrap();

        assert!(
            runtime
                .script_element_request_csp_violation(&allowed_url)
                .is_none()
        );
    }

    #[test]
    fn csp_host_source_path_allows_directory_descendants_only() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src https://cdn.example.test/trusted/">
            "#,
        );
        let allowed = Url::parse("https://cdn.example.test/trusted/module.wasm").unwrap();
        let blocked = Url::parse("https://cdn.example.test/evil/module.wasm").unwrap();
        let percent_encoded_slash =
            Url::parse("https://cdn.example.test/trusted%2Fevil.wasm").unwrap();

        assert!(
            runtime
                .script_element_request_csp_violation(&allowed)
                .is_none()
        );
        assert!(
            runtime
                .script_element_request_csp_violation(&blocked)
                .is_some()
        );
        assert!(
            runtime
                .script_element_request_csp_violation(&percent_encoded_slash)
                .is_some()
        );
    }

    #[test]
    fn csp_host_source_path_without_trailing_slash_requires_exact_path() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src https://cdn.example.test/trusted/module.wasm">
            "#,
        );
        let allowed = Url::parse("https://cdn.example.test/trusted/module.wasm").unwrap();
        let blocked = Url::parse("https://cdn.example.test/trusted/module.wasm.extra").unwrap();

        assert!(
            runtime
                .script_element_request_csp_violation(&allowed)
                .is_none()
        );
        assert!(
            runtime
                .script_element_request_csp_violation(&blocked)
                .is_some()
        );
    }

    #[test]
    fn csp_host_source_without_path_still_allows_any_path() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src https://cdn.example.test">
            "#,
        );
        let allowed = Url::parse("https://cdn.example.test/any/module.wasm").unwrap();

        assert!(
            runtime
                .script_element_request_csp_violation(&allowed)
                .is_none()
        );
    }

    #[test]
    fn csp_wildcard_host_source_allows_subdomains_but_not_bare_host() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src https://*.example.test">
            "#,
        );
        let subdomain = Url::parse("https://cdn.example.test/module.wasm").unwrap();
        let nested_subdomain = Url::parse("https://a.b.example.test/module.wasm").unwrap();
        let bare_host = Url::parse("https://example.test/module.wasm").unwrap();
        let similar_suffix = Url::parse("https://badexample.test/module.wasm").unwrap();

        assert!(
            runtime
                .script_element_request_csp_violation(&subdomain)
                .is_none()
        );
        assert!(
            runtime
                .script_element_request_csp_violation(&nested_subdomain)
                .is_none()
        );
        assert!(
            runtime
                .script_element_request_csp_violation(&bare_host)
                .is_some()
        );
        assert!(
            runtime
                .script_element_request_csp_violation(&similar_suffix)
                .is_some()
        );
    }

    #[test]
    fn csp_exact_host_source_still_allows_bare_host() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src https://example.test">
            "#,
        );
        let bare_host = Url::parse("https://example.test/module.wasm").unwrap();
        let subdomain = Url::parse("https://cdn.example.test/module.wasm").unwrap();

        assert!(
            runtime
                .script_element_request_csp_violation(&bare_host)
                .is_none()
        );
        assert!(
            runtime
                .script_element_request_csp_violation(&subdomain)
                .is_some()
        );
    }

    #[test]
    fn csp_host_source_without_port_does_not_allow_non_default_ports() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src https://cdn.example.test">
            "#,
        );
        let blocked = Url::parse("https://cdn.example.test:444/module.wasm").unwrap();

        assert!(
            runtime
                .script_element_request_csp_violation(&blocked)
                .is_some()
        );
    }

    #[test]
    fn csp_host_source_rejects_query_without_path() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src https://cdn.example.test?ignored">
            "#,
        );
        let blocked = Url::parse("https://cdn.example.test/module.wasm").unwrap();

        assert!(
            runtime
                .script_element_request_csp_violation(&blocked)
                .is_some()
        );
    }

    #[test]
    fn csp_script_element_report_only_uses_shared_url_helper() {
        let runtime = runtime_with_response_csp_report_only("default-src 'none'");
        let request_url = Url::parse("https://example.test/app.js").unwrap();

        let violation = runtime
            .script_element_request_csp_report_only_violation(&request_url)
            .expect("report-only default-src should report external script violation");
        assert_eq!(violation.effective_directive, "script-src-elem");
        assert_eq!(violation.blocked_uri, request_url.to_string());
        assert_eq!(
            violation.disposition,
            ContentSecurityPolicyDisposition::Report
        );
    }

    #[test]
    fn csp_meta_script_report_uri_is_not_a_network_report_endpoint() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="script-src 'none'; report-uri /csp-report">
            "#,
        );
        let request_url = Url::parse("https://example.test/app.js").unwrap();

        let violation = runtime
            .script_element_request_csp_violation(&request_url)
            .expect("meta script-src should block external script");
        assert_eq!(violation.effective_directive, "script-src-elem");
        assert!(violation.report_uri_endpoints.is_empty());
    }

    #[test]
    fn csp_response_report_to_attaches_reporting_api_endpoint() {
        let mut runtime =
            runtime_with_response_csp("connect-src 'none'; report-uri /legacy; report-to csp");
        runtime.set_content_security_reporting_endpoints(reporting_endpoints());
        let request_url = Url::parse("https://api.example.test/data.json").unwrap();

        let check = runtime.document_connect_csp_check_with_redirect_status(
            &request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        );
        let violation = check
            .enforced_violation()
            .expect("connect-src none should block request");

        assert!(violation.report_uri_endpoints.is_empty());
        assert_eq!(
            violation.report_to_endpoints,
            vec!["https://example.test/reports/csp".to_owned()]
        );
    }

    #[test]
    fn csp_response_report_to_attaches_inline_and_wasm_reporting_api_endpoint() {
        let mut runtime = runtime_with_response_csp("script-src 'none'; report-to csp");
        runtime.set_content_security_reporting_endpoints(reporting_endpoints());

        let inline = runtime
            .inline_script_csp_violation_for_child_document(
                Some(runtime.document_handle()),
                runtime.document_url(),
                &["script-src 'none'; report-to csp".to_owned()],
                &reporting_endpoints(),
            )
            .expect("inline script should violate script-src none");
        let wasm = runtime
            .wasm_eval_csp_violation()
            .expect("wasm eval should violate script-src none");

        assert_eq!(
            inline.report_to_endpoints,
            vec!["https://example.test/reports/csp".to_owned()]
        );
        assert_eq!(
            wasm.report_to_endpoints,
            vec!["https://example.test/reports/csp".to_owned()]
        );
    }

    #[test]
    fn csp_wasm_eval_uses_script_src_or_default_src() {
        let default_blocked = runtime_with_response_csp("default-src 'self' 'unsafe-inline'");
        let default_violation = default_blocked
            .wasm_eval_csp_violation()
            .expect("default-src should block wasm eval when unsafe wasm sources are absent");
        assert_eq!(default_violation.blocked_uri, "wasm-eval");
        assert_eq!(
            default_violation.original_policy,
            "default-src 'self' 'unsafe-inline'"
        );

        let script_blocked =
            runtime_with_response_csp("script-src 'self' 'unsafe-inline'; default-src *");
        assert!(script_blocked.wasm_eval_csp_violation().is_some());
    }

    #[test]
    fn csp_report_only_wasm_eval_returns_report_disposition() {
        let runtime = runtime_with_response_csp_report_only("script-src 'self' 'unsafe-inline'");
        let violation = runtime
            .wasm_eval_csp_report_only_violation()
            .expect("report-only script-src without wasm unsafe eval should report");

        assert_eq!(violation.blocked_uri, "wasm-eval");
        assert_eq!(
            violation.disposition,
            ContentSecurityPolicyDisposition::Report
        );
    }

    #[test]
    fn csp_document_connect_uses_connect_src_before_default_src() {
        let runtime = runtime_with_response_csp("connect-src 'self'; default-src *");
        let same_origin = Url::parse("https://example.test/api").unwrap();
        let cross_origin = Url::parse("https://api.example.test/data").unwrap();

        let same_origin_check = runtime.document_connect_csp_check_with_redirect_status(
            &same_origin,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        );
        assert!(same_origin_check.enforced_violation().is_none());

        let check = runtime.document_connect_csp_check_with_redirect_status(
            &cross_origin,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        );
        let violation = check
            .enforced_violation()
            .expect("connect-src should block cross-origin document fetch");
        assert_eq!(violation.effective_directive, "connect-src");
        assert_eq!(violation.blocked_uri, cross_origin.to_string());
        assert_eq!(
            violation.disposition,
            ContentSecurityPolicyDisposition::Enforce
        );
    }

    #[test]
    fn csp_document_connect_falls_back_to_default_src() {
        let runtime = runtime_with_response_csp("default-src 'self'");
        let blocked = Url::parse("https://api.example.test/data").unwrap();

        let check = runtime.document_connect_csp_check_with_redirect_status(
            &blocked,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        );
        let violation = check
            .enforced_violation()
            .expect("default-src should apply when connect-src is absent");
        assert_eq!(violation.effective_directive, "connect-src");
        assert_eq!(violation.original_policy, "default-src 'self'");
    }

    #[test]
    fn csp_document_connect_report_only_returns_report_disposition() {
        let runtime = runtime_with_response_csp_report_only("connect-src 'none'");
        let request_url = Url::parse("https://example.test/api").unwrap();

        let check = runtime.document_connect_csp_check_with_redirect_status(
            &request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        );
        let violation = check
            .report_only_violation()
            .expect("report-only connect-src should produce a report violation");
        assert_eq!(violation.effective_directive, "connect-src");
        assert_eq!(violation.blocked_uri, request_url.to_string());
        assert_eq!(
            violation.disposition,
            ContentSecurityPolicyDisposition::Report
        );
    }

    #[test]
    fn csp_document_connect_check_keeps_report_only_and_enforce_together() {
        let mut runtime = runtime_with_response_csp("connect-src 'none'");
        runtime.set_response_content_security_report_only_policies(&[String::from(
            "connect-src 'none'",
        )]);
        let request_url = Url::parse("https://api.example.test/redirected").unwrap();

        let check = runtime.document_connect_csp_check_with_redirect_status(
            &request_url,
            ContentSecurityPolicyRedirectStatus::FollowedRedirect,
        );
        let report_only = check
            .report_only_violation()
            .expect("report-only connect-src should be included in the combined check");
        let enforced = check
            .enforced_violation()
            .expect("enforced connect-src should be included in the combined check");

        assert_eq!(
            report_only.disposition,
            ContentSecurityPolicyDisposition::Report
        );
        assert_eq!(
            enforced.disposition,
            ContentSecurityPolicyDisposition::Enforce
        );
    }

    #[test]
    fn csp_document_connect_enforce_includes_meta_policies() {
        let runtime = runtime_for_html(
            r#"<!doctype html>
            <meta http-equiv="Content-Security-Policy" content="connect-src 'none'">
            "#,
        );
        let request_url = Url::parse("https://example.test/api").unwrap();

        let check = runtime.document_connect_csp_check_with_redirect_status(
            &request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        );
        let violation = check
            .enforced_violation()
            .expect("meta connect-src should block document connect request");
        assert_eq!(violation.original_policy, "connect-src 'none'");
    }

    #[test]
    fn csp_frame_navigation_uses_frame_src_with_child_src_fallback() {
        let blocked_url = Url::parse("https://frames.example.test/child.html").unwrap();
        let frame_src_runtime = runtime_with_response_csp("frame-src 'none'; child-src *");
        let frame_src_violation = frame_src_runtime
            .document_frame_csp_violation(&blocked_url)
            .expect("frame-src should block iframe navigation before child-src fallback");
        assert_eq!(frame_src_violation.effective_directive, "frame-src");
        assert_eq!(
            frame_src_violation.original_policy,
            "frame-src 'none'; child-src *"
        );

        let child_src_runtime = runtime_with_response_csp("child-src 'none'");
        let child_src_violation = child_src_runtime
            .document_frame_csp_violation(&blocked_url)
            .expect("child-src should block iframe navigation when frame-src is absent");
        assert_eq!(child_src_violation.effective_directive, "frame-src");
        assert_eq!(child_src_violation.original_policy, "child-src 'none'");
    }

    #[test]
    fn csp_wasm_eval_allows_wasm_unsafe_eval_and_unsafe_eval() {
        let wasm_allowed =
            runtime_with_response_csp("default-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'");
        assert!(wasm_allowed.wasm_eval_csp_violation().is_none());

        let unsafe_eval_allowed =
            runtime_with_response_csp("script-src 'self' 'unsafe-inline' 'unsafe-eval'");
        assert!(unsafe_eval_allowed.wasm_eval_csp_violation().is_none());
    }

    #[test]
    fn csp_response_header_blocks_child_document_script_requests() {
        let runtime = runtime_for_html("<!doctype html>");
        let document_url = Url::parse("https://example.test/popup.html").unwrap();
        let response_policies = vec!["script-src 'self'".to_owned()];
        let blocked = Url::parse("https://cdn.example.test/popup.js").unwrap();
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();

        let violation = runtime
            .script_element_request_csp_violation_for_child_document(
                None,
                &document_url,
                &response_policies,
                &reporting_endpoints,
                &blocked,
            )
            .expect("child response CSP should block cross-origin script request");

        assert_eq!(violation.effective_directive, "script-src-elem");
        assert_eq!(violation.blocked_uri, blocked.to_string());
        assert_eq!(violation.document_uri, document_url.to_string());
    }

    #[test]
    fn csp_response_header_blocks_child_inline_scripts_without_unsafe_inline() {
        let runtime = runtime_for_html("<!doctype html>");
        let document_url = Url::parse("https://example.test/popup.html").unwrap();
        let blocking_policies = vec!["script-src 'self'".to_owned()];
        let allowing_policies = vec!["script-src 'unsafe-inline'".to_owned()];
        let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();

        let violation = runtime
            .inline_script_csp_violation_for_child_document(
                None,
                &document_url,
                &blocking_policies,
                &reporting_endpoints,
            )
            .expect("child response CSP should block inline script without unsafe-inline");

        assert_eq!(violation.effective_directive, "script-src-elem");
        assert_eq!(violation.blocked_uri, "inline");
        assert!(
            runtime
                .inline_script_csp_violation_for_child_document(
                    None,
                    &document_url,
                    &allowing_policies,
                    &reporting_endpoints,
                )
                .is_none()
        );
    }

    #[test]
    fn csp_response_header_extraction_is_case_insensitive() {
        assert_eq!(
            response_content_security_policies_from_headers(&[
                (
                    "Content-Security-Policy".to_owned(),
                    " default-src 'self' ".to_owned()
                ),
                (
                    "content-security-policy-report-only".to_owned(),
                    "default-src 'none'".to_owned()
                ),
                ("X-Other".to_owned(), "ignored".to_owned()),
            ]),
            vec!["default-src 'self'"]
        );
        assert_eq!(
            response_content_security_report_only_policies_from_headers(&[
                (
                    "Content-Security-Policy".to_owned(),
                    "default-src 'self'".to_owned()
                ),
                (
                    "content-security-policy-report-only".to_owned(),
                    " default-src 'none' ".to_owned()
                ),
                ("X-Other".to_owned(), "ignored".to_owned()),
            ]),
            vec!["default-src 'none'"]
        );
    }
}
