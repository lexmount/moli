use super::{JsContextHost, OwnerDispatchScope};
use crate::{
    content_security_policy::{
        ContentSecurityPolicyNonUrlKind, ContentSecurityPolicyRedirectStatus,
        ContentSecurityPolicyReportingEndpoints, ContentSecurityPolicyScriptElementRequest,
        ContentSecurityPolicyViolationEventFields, TrustedTypesForScriptRequirements,
    },
    context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
    document_runtime::{
        DocumentContentSecurityPolicyCheck, DocumentContentSecurityPolicyViolation,
        DocumentPolicyContainer, DocumentSubresourceCspKind, DomHandle,
        create_content_security_policy_violation_event,
    },
    native_bridge::{
        active_child_window_handle, active_lightweight_popup_id,
        child_window_handle_from_marker_data, entered_child_window_handle,
    },
    util::get_private_value,
};

#[derive(Debug)]
#[must_use = "the caller must stop a request when CSP blocks it"]
pub(crate) enum DocumentCspOutcome {
    Allowed,
    Blocked(DocumentContentSecurityPolicyViolation),
    SkippedNonTopContext,
}

#[derive(Debug, Clone)]
struct OwnerDocumentPolicySnapshot {
    document_handle: Option<DomHandle>,
    document_url: url::Url,
    policy_container: DocumentPolicyContainer,
}

const JAVASCRIPT_URL_TRUSTED_TYPES_SINK: &str = "Location href";

fn policy_child_window_handle(scope: &mut v8::PinScope<'_, '_>) -> Option<DomHandle> {
    if let Some(handle) = active_child_window_handle(scope) {
        return Some(handle);
    }
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| child_window_handle_from_marker_data(scope, value))
}

fn policy_owner_dispatch_scope(scope: &mut v8::PinScope<'_, '_>) -> OwnerDispatchScope {
    if let Some(handle) = policy_child_window_handle(scope) {
        return OwnerDispatchScope::Child(handle);
    }
    if let Some(popup_id) = active_lightweight_popup_id(scope) {
        return OwnerDispatchScope::LightweightPopup(popup_id);
    }
    OwnerDispatchScope::Top
}

impl DocumentCspOutcome {
    pub(crate) fn blocks_request(&self) -> bool {
        matches!(self, Self::Blocked(_))
    }

    pub(crate) fn into_blocking_violation(self) -> Option<DocumentContentSecurityPolicyViolation> {
        match self {
            Self::Blocked(violation) => Some(violation),
            Self::Allowed | Self::SkippedNonTopContext => None,
        }
    }
}

impl JsContextHost {
    fn owner_document_policy_snapshot(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<OwnerDocumentPolicySnapshot> {
        match owner {
            OwnerDispatchScope::Top => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                let runtime = unsafe { &*self.runtime };
                Some(OwnerDocumentPolicySnapshot {
                    document_handle: Some(runtime.document_handle()),
                    document_url: runtime.document_url().clone(),
                    policy_container: runtime.document_policy_container().clone(),
                })
            }
            OwnerDispatchScope::Child(handle) => {
                let mut policy_container =
                    self.child_browsing_context_policy_container_snapshot(handle)?;
                policy_container.response_content_security_policies =
                    self.child_effective_enforced_content_security_policies(handle);
                policy_container.response_content_security_report_only_policies =
                    self.child_effective_response_content_security_report_only_policies(handle);
                policy_container.content_security_reporting_endpoints =
                    self.child_effective_content_security_reporting_endpoints(handle);
                Some(OwnerDocumentPolicySnapshot {
                    document_handle: self.child_browsing_context_document_handle(handle),
                    document_url: self.child_browsing_context_current_url(handle)?,
                    policy_container,
                })
            }
            OwnerDispatchScope::LightweightPopup(popup_id) => Some(OwnerDocumentPolicySnapshot {
                document_handle: self.lightweight_popup_document_handle(popup_id),
                document_url: self.lightweight_popup_document_url(popup_id)?,
                policy_container: self.lightweight_popup_policy_container(popup_id)?.clone(),
            }),
        }
    }

    pub(crate) fn document_policy_container(
        &self,
    ) -> &crate::document_runtime::DocumentPolicyContainer {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.document_policy_container()
    }

    pub(crate) fn document_connect_policy_snapshot_for_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<crate::document_runtime::DocumentConnectPolicySnapshot> {
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        Some(
            crate::document_runtime::DocumentConnectPolicySnapshot::from_policy_container(
                &snapshot.policy_container,
            ),
        )
    }

    pub(crate) fn document_permissions_policy_for_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<crate::permissions_policy::DocumentPermissionsPolicy> {
        match owner {
            OwnerDispatchScope::Top => Some(self.document_policy_container().permissions_policy),
            OwnerDispatchScope::Child(handle) => self
                .frame_owner_current_child_snapshot(handle)
                .map(|snapshot| {
                    snapshot
                        .settings
                        .document_policy_container
                        .permissions_policy
                }),
            OwnerDispatchScope::LightweightPopup(popup_id) => self
                .lightweight_popup_policy_container(popup_id)
                .map(|policy| policy.permissions_policy),
        }
    }

    pub(crate) fn document_permissions_policy_for_document_handle(
        &self,
        document: DomHandle,
    ) -> Option<crate::permissions_policy::DocumentPermissionsPolicy> {
        if document == self.document_handle() {
            return Some(self.document_policy_container().permissions_policy);
        }
        if let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document) {
            return self
                .lightweight_popup_policy_container(popup_id)
                .map(|policy| policy.permissions_policy);
        }
        let child_handle = self.child_browsing_context_host_for_document_handle(document)?;
        let snapshot = self.frame_owner_current_child_snapshot(child_handle)?;
        (snapshot.document_handle == document).then_some(
            snapshot
                .settings
                .document_policy_container
                .permissions_policy,
        )
    }

    pub(crate) fn trusted_types_for_script_requirements_for_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<TrustedTypesForScriptRequirements> {
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        Some(
            unsafe { &*self.runtime }.trusted_types_for_script_requirements_for_document(
                snapshot.document_handle,
                &snapshot.policy_container.response_content_security_policies,
                &snapshot
                    .policy_container
                    .response_content_security_report_only_policies,
                &snapshot
                    .policy_container
                    .content_security_reporting_endpoints,
            ),
        )
    }

    fn trusted_types_sink_csp_violations_for_owner(
        &self,
        owner: OwnerDispatchScope,
        sink: &str,
        sample: &str,
    ) -> Vec<DocumentContentSecurityPolicyViolation> {
        let Some(snapshot) = self.owner_document_policy_snapshot(owner) else {
            return Vec::new();
        };
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.trusted_types_sink_csp_violations_for_document(
            snapshot.document_handle,
            &snapshot.document_url,
            &snapshot.policy_container.response_content_security_policies,
            &snapshot
                .policy_container
                .response_content_security_report_only_policies,
            &snapshot
                .policy_container
                .content_security_reporting_endpoints,
            sink,
            sample,
        )
    }

    /// Run the target Document checks immediately before a `javascript:` URL
    /// executes. Source-context admission checks may already have happened at
    /// the navigation API boundary; this is the Chromium-style target check
    /// shared by top-level, child-frame, and lightweight-popup executors.
    pub(crate) fn prepare_javascript_url_source_for_execution<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        source: &str,
    ) -> Option<String> {
        let csp_source = format!("javascript:{source}");
        if !self.allows_inline_javascript_navigation_by_csp(scope, owner, &csp_source) {
            return None;
        }

        let requirements = self.trusted_types_for_script_requirements_for_owner(owner)?;
        let check = crate::context_bootstrap::check_javascript_url_trusted_types(
            scope,
            source,
            requirements,
        );
        if check.violated {
            let host_ptr: *mut JsContextHost = self;
            self.dispatch_trusted_types_sink_csp_violation_event_for_owner_without_stack_best_effort(
                scope,
                host_ptr,
                owner,
                JAVASCRIPT_URL_TRUSTED_TYPES_SINK,
                source,
            );
        }
        check.source
    }

    pub(crate) fn cross_origin_embedder_policy(
        &self,
    ) -> crate::cross_origin_isolation::CrossOriginEmbedderPolicy {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.cross_origin_embedder_policy()
    }

    pub(crate) fn document_isolation_policy(
        &self,
    ) -> crate::cross_origin_isolation::DocumentIsolationPolicy {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.document_isolation_policy()
    }

    pub(crate) fn cross_origin_isolated(&self) -> bool {
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.cross_origin_isolated()
    }

    pub(crate) fn check_top_document_subresource_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        request_url: &url::Url,
        kind: DocumentSubresourceCspKind,
    ) -> DocumentCspOutcome {
        if policy_owner_dispatch_scope(scope) != OwnerDispatchScope::Top {
            return DocumentCspOutcome::SkippedNonTopContext;
        }
        let (report_only_violation, enforced_violation) = self
            .document_subresource_csp_check(request_url, kind)
            .into_violations();
        let host_ptr: *mut JsContextHost = self;
        if let Some(violation) = report_only_violation {
            self.dispatch_content_security_policy_violation_event_best_effort(
                scope, host_ptr, &violation,
            );
        }
        let Some(violation) = enforced_violation else {
            return DocumentCspOutcome::Allowed;
        };
        self.dispatch_content_security_policy_violation_event_best_effort(
            scope, host_ptr, &violation,
        );
        DocumentCspOutcome::Blocked(violation)
    }

    pub(crate) fn owner_dispatch_scope_for_node(
        &self,
        handle: DomHandle,
    ) -> Option<OwnerDispatchScope> {
        if let Some(popup_id) = self.lightweight_popup_id_for_node_owner_document(handle) {
            return Some(OwnerDispatchScope::LightweightPopup(popup_id));
        }
        let node = self.dom_host().node(handle)?;
        let owner_document = if node.is_document() {
            handle
        } else {
            node.owner_document()?
        };
        if owner_document == self.document_handle() {
            return Some(OwnerDispatchScope::Top);
        }
        self.child_browsing_context_host_for_document_handle(owner_document)
            .map(OwnerDispatchScope::Child)
    }

    pub(crate) fn allows_inline_event_handler_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        source: &str,
    ) -> bool {
        self.allows_inline_source_by_csp(
            scope,
            owner,
            ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
            source,
        )
    }

    pub(crate) fn allows_inline_script_element_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        script: DomHandle,
        source: &str,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> bool {
        let Some(check) = self.inline_script_element_csp_check_for_owner(owner, source, request)
        else {
            return true;
        };
        let (mut report_only_violation, mut enforced_violation) = check.into_violations();
        if owner == OwnerDispatchScope::Top
            && let Some(position) = unsafe { &*self.runtime }.parser_script_start_position(script)
        {
            let line = i32::try_from(position.line).unwrap_or(i32::MAX);
            let column = i32::try_from(position.column).unwrap_or(i32::MAX);
            if let Some(violation) = report_only_violation.as_mut() {
                violation.line_number = line;
                violation.column_number = column;
            }
            if let Some(violation) = enforced_violation.as_mut() {
                violation.line_number = line;
                violation.column_number = column;
            }
        }
        let host_ptr: *mut JsContextHost = self;
        if let Some(violation) = report_only_violation {
            self.dispatch_content_security_policy_violation_event_for_element_owner_best_effort(
                scope, host_ptr, owner, script, &violation,
            );
        }
        let Some(violation) = enforced_violation else {
            return true;
        };
        self.dispatch_content_security_policy_violation_event_for_element_owner_best_effort(
            scope, host_ptr, owner, script, &violation,
        );
        false
    }

    pub(crate) fn allows_inline_javascript_navigation_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        source: &str,
    ) -> bool {
        self.allows_inline_source_by_csp(
            scope,
            owner,
            ContentSecurityPolicyNonUrlKind::DocumentInlineNavigation,
            source,
        )
    }

    fn allows_inline_source_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        kind: ContentSecurityPolicyNonUrlKind,
        source: &str,
    ) -> bool {
        let Some(check) = self.inline_source_csp_check_for_owner(owner, kind, source) else {
            // Deliberately fail open only while a child or lightweight popup
            // has no active document URL/policy context (for example during a
            // navigation swap). CSP is document-scoped: applying the top-level
            // or the previous document's policy here would enforce the wrong
            // owner. Committed documents must always produce a check.
            return true;
        };
        let (report_only_violation, enforced_violation) = check.into_violations();
        let host_ptr: *mut JsContextHost = self;
        if let Some(violation) = report_only_violation {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        let Some(violation) = enforced_violation else {
            return true;
        };
        self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
            scope, host_ptr, owner, &violation,
        );
        false
    }

    fn inline_source_csp_check_for_owner(
        &self,
        owner: OwnerDispatchScope,
        kind: ContentSecurityPolicyNonUrlKind,
        source: &str,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        Some(
            unsafe { &*self.runtime }.inline_source_csp_check_for_document(
                snapshot.document_handle,
                &snapshot.document_url,
                &snapshot.policy_container.response_content_security_policies,
                &snapshot
                    .policy_container
                    .response_content_security_report_only_policies,
                &snapshot
                    .policy_container
                    .content_security_reporting_endpoints,
                kind,
                source,
            ),
        )
    }

    fn inline_script_element_csp_check_for_owner(
        &self,
        owner: OwnerDispatchScope,
        source: &str,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        Some(
            unsafe { &*self.runtime }.inline_script_element_csp_check_for_document(
                snapshot.document_handle,
                &snapshot.document_url,
                &snapshot.policy_container.response_content_security_policies,
                &snapshot
                    .policy_container
                    .response_content_security_report_only_policies,
                &snapshot
                    .policy_container
                    .content_security_reporting_endpoints,
                source,
                request,
            ),
        )
    }

    pub(crate) fn entered_owner_dispatch_scope(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> OwnerDispatchScope {
        if let Some(handle) = entered_child_window_handle(scope) {
            return OwnerDispatchScope::Child(handle);
        }
        if let Some(popup_id) = active_lightweight_popup_id(scope) {
            return OwnerDispatchScope::LightweightPopup(popup_id);
        }
        OwnerDispatchScope::Top
    }

    pub(crate) fn check_document_connect_csp_for_owner<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        document_url: &url::Url,
        request_url: &url::Url,
    ) -> DocumentCspOutcome {
        self.check_document_connect_csp_for_owner_with_redirect_status(
            scope,
            owner,
            document_url,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
    }

    pub(crate) fn check_document_connect_csp_for_owner_with_redirect_status<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        document_url: &url::Url,
        request_url: &url::Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> DocumentCspOutcome {
        let Some(check) = self.document_connect_csp_check_for_owner_with_redirect_status(
            owner,
            document_url,
            request_url,
            redirect_status,
        ) else {
            return DocumentCspOutcome::Allowed;
        };
        let (report_only_violation, enforced_violation) = check.into_violations();
        let host_ptr: *mut JsContextHost = self;
        if let Some(violation) = report_only_violation {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        let Some(violation) = enforced_violation else {
            return DocumentCspOutcome::Allowed;
        };
        self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
            scope, host_ptr, owner, &violation,
        );
        DocumentCspOutcome::Blocked(violation)
    }

    fn document_connect_csp_check_for_owner_with_redirect_status(
        &self,
        owner: OwnerDispatchScope,
        document_url: &url::Url,
        request_url: &url::Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        match owner {
            OwnerDispatchScope::Top => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                Some(
                    unsafe { &*self.runtime }.document_connect_csp_check_with_redirect_status(
                        request_url,
                        redirect_status,
                    ),
                )
            }
            OwnerDispatchScope::Child(handle) => Some(
                self.document_connect_csp_check_for_child_document_with_redirect_status(
                    handle,
                    document_url,
                    request_url,
                    redirect_status,
                ),
            ),
            OwnerDispatchScope::LightweightPopup(popup_id) => self
                .document_connect_csp_check_for_lightweight_popup_with_redirect_status(
                    popup_id,
                    document_url,
                    request_url,
                    redirect_status,
                ),
        }
    }

    pub(crate) fn document_connect_csp_allows_for_owner(
        &self,
        owner: OwnerDispatchScope,
        document_url: &url::Url,
        request_url: &url::Url,
    ) -> bool {
        self.document_connect_csp_check_for_owner_with_redirect_status(
            owner,
            document_url,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .map(|check| check.into_violations().1.is_none())
        .unwrap_or(true)
    }

    fn document_connect_csp_check_for_lightweight_popup_with_redirect_status(
        &self,
        popup_id: u64,
        document_url: &url::Url,
        request_url: &url::Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        let policy_container = self.lightweight_popup_policy_container(popup_id)?;
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        Some(
            unsafe { &*self.runtime }.document_connect_csp_check_for_document_with_redirect_status(
                self.lightweight_popup_document_handle(popup_id),
                document_url,
                &policy_container.response_content_security_policies,
                &policy_container.response_content_security_report_only_policies,
                &policy_container.content_security_reporting_endpoints,
                request_url,
                redirect_status,
            ),
        )
    }

    fn document_connect_csp_check_for_child_document_with_redirect_status(
        &self,
        handle: DomHandle,
        document_url: &url::Url,
        request_url: &url::Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> DocumentContentSecurityPolicyCheck {
        let response_policies = self.child_response_content_security_policies(handle);
        let response_report_only_policies =
            self.child_response_content_security_report_only_policies(handle);
        let response_reporting_endpoints =
            self.child_effective_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.document_connect_csp_check_for_document_with_redirect_status(
            self.child_browsing_context_document_handle(handle),
            document_url,
            response_policies,
            response_report_only_policies,
            &response_reporting_endpoints,
            request_url,
            redirect_status,
        )
    }

    pub(crate) fn frame_navigation_csp_violation(
        &self,
        frame_handle: DomHandle,
        request_url: &url::Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let owner = self.owner_dispatch_scope_for_node(frame_handle)?;
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.document_frame_csp_violation_for_document(
            snapshot.document_handle,
            &snapshot.document_url,
            &snapshot.policy_container.response_content_security_policies,
            &snapshot
                .policy_container
                .content_security_reporting_endpoints,
            request_url,
        )
    }

    pub(crate) fn frame_navigation_csp_report_only_violation(
        &self,
        frame_handle: DomHandle,
        request_url: &url::Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let owner = self.owner_dispatch_scope_for_node(frame_handle)?;
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.document_frame_csp_report_only_violation_for_document(
            &snapshot.document_url,
            &snapshot
                .policy_container
                .response_content_security_report_only_policies,
            &snapshot
                .policy_container
                .content_security_reporting_endpoints,
            request_url,
        )
    }

    fn child_effective_enforced_content_security_policies(
        &self,
        child_handle: DomHandle,
    ) -> Vec<String> {
        let mut policies = Vec::new();
        if self
            .child_browsing_contexts
            .get(&child_handle)
            .is_some_and(|entry| entry.content_security_policy_inherited())
        {
            policies.extend(self.parent_effective_enforced_content_security_policies(child_handle));
        }
        policies.extend(
            self.child_browsing_contexts
                .get(&child_handle)
                .map(|entry| entry.response_content_security_policies().to_vec())
                .unwrap_or_default(),
        );
        policies
    }

    fn child_effective_response_content_security_report_only_policies(
        &self,
        child_handle: DomHandle,
    ) -> Vec<String> {
        let mut policies = Vec::new();
        if self
            .child_browsing_contexts
            .get(&child_handle)
            .is_some_and(|entry| entry.content_security_policy_inherited())
        {
            policies.extend(
                self.parent_effective_response_content_security_report_only_policies(child_handle),
            );
        }
        policies.extend(
            self.child_browsing_contexts
                .get(&child_handle)
                .map(|entry| {
                    entry
                        .response_content_security_report_only_policies()
                        .to_vec()
                })
                .unwrap_or_default(),
        );
        policies
    }

    fn parent_effective_enforced_content_security_policies(
        &self,
        child_handle: DomHandle,
    ) -> Vec<String> {
        match self.child_browsing_context_parent_handle(child_handle) {
            Some(parent) => {
                let mut policies = self.child_effective_enforced_content_security_policies(parent);
                if let Some(document) = self.child_browsing_context_document_handle(parent) {
                    // SAFETY: JsContextHost is owned by the ScriptVm that owns this
                    // DocumentRuntime.
                    policies.extend(
                        unsafe { &*self.runtime }
                            .meta_content_security_policy_strings_for_document(document),
                    );
                }
                policies
            }
            None => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this
                // DocumentRuntime.
                let runtime = unsafe { &*self.runtime };
                let mut policies = runtime.response_content_security_policies().to_vec();
                policies.extend(
                    runtime.meta_content_security_policy_strings_for_document(
                        runtime.document_handle(),
                    ),
                );
                policies
            }
        }
    }

    fn parent_effective_response_content_security_report_only_policies(
        &self,
        child_handle: DomHandle,
    ) -> Vec<String> {
        match self.child_browsing_context_parent_handle(child_handle) {
            Some(parent) => {
                self.child_effective_response_content_security_report_only_policies(parent)
            }
            None => unsafe { &*self.runtime }
                .response_content_security_report_only_policies()
                .to_vec(),
        }
    }

    fn child_effective_content_security_reporting_endpoints(
        &self,
        child_handle: DomHandle,
    ) -> ContentSecurityPolicyReportingEndpoints {
        let inherits_parent = self
            .child_browsing_contexts
            .get(&child_handle)
            .is_some_and(|entry| entry.content_security_policy_inherited());
        let has_own_response_policies = self
            .child_browsing_contexts
            .get(&child_handle)
            .is_some_and(|entry| entry.has_response_content_security_policies());
        if inherits_parent && !has_own_response_policies {
            return self.parent_effective_content_security_reporting_endpoints(child_handle);
        }
        self.child_browsing_contexts
            .get(&child_handle)
            .map(|entry| entry.content_security_reporting_endpoints())
            .unwrap_or_default()
    }

    fn parent_effective_content_security_reporting_endpoints(
        &self,
        child_handle: DomHandle,
    ) -> ContentSecurityPolicyReportingEndpoints {
        match self.child_browsing_context_parent_handle(child_handle) {
            Some(parent) => self.child_effective_content_security_reporting_endpoints(parent),
            None => unsafe { &*self.runtime }
                .content_security_reporting_endpoints()
                .clone(),
        }
    }

    pub(crate) fn allows_eval_code_generation_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        allow_trusted_types_eval: bool,
        source: Option<&str>,
    ) -> bool {
        let owner = policy_owner_dispatch_scope(scope);
        let Some(check) = self.non_url_csp_check_for_owner(
            owner,
            if allow_trusted_types_eval {
                ContentSecurityPolicyNonUrlKind::TrustedTypesEval
            } else {
                ContentSecurityPolicyNonUrlKind::Eval
            },
            source,
        ) else {
            return true;
        };
        self.apply_code_generation_csp_check(scope, owner, check, true)
    }

    pub(crate) fn allows_wasm_code_generation_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> bool {
        let owner = policy_owner_dispatch_scope(scope);
        let Some(check) = self.non_url_csp_check_for_owner(
            owner,
            ContentSecurityPolicyNonUrlKind::WasmEval,
            None,
        ) else {
            return true;
        };
        self.apply_code_generation_csp_check(scope, owner, check, false)
    }

    fn non_url_csp_check_for_owner(
        &self,
        owner: OwnerDispatchScope,
        kind: ContentSecurityPolicyNonUrlKind,
        source: Option<&str>,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        Some(
            unsafe { &*self.runtime }.non_url_csp_check_for_document(
                snapshot.document_handle,
                &snapshot.document_url,
                &snapshot.policy_container.response_content_security_policies,
                &snapshot
                    .policy_container
                    .response_content_security_report_only_policies,
                &snapshot
                    .policy_container
                    .content_security_reporting_endpoints,
                kind,
                source,
            ),
        )
    }

    fn apply_code_generation_csp_check<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        check: DocumentContentSecurityPolicyCheck,
        include_call_location: bool,
    ) -> bool {
        let (mut report_only_violation, mut enforced_violation) = check.into_violations();
        if report_only_violation.is_none() && enforced_violation.is_none() {
            return true;
        }
        if include_call_location
            && let Some((source_file, line_number, column_number)) =
                current_script_violation_location(scope)
        {
            for violation in [&mut report_only_violation, &mut enforced_violation]
                .into_iter()
                .flatten()
            {
                violation.source_file = source_file.clone();
                violation.line_number = line_number;
                violation.column_number = column_number;
            }
        }
        let host_ptr: *mut JsContextHost = self;
        if let Some(violation) = report_only_violation {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        let Some(violation) = enforced_violation else {
            return true;
        };
        self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
            scope, host_ptr, owner, &violation,
        );
        false
    }

    pub(crate) fn child_wasm_eval_csp_violation(
        &self,
        handle: DomHandle,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        self.non_url_csp_check_for_owner(
            OwnerDispatchScope::Child(handle),
            ContentSecurityPolicyNonUrlKind::WasmEval,
            None,
        )?
        .into_violations()
        .1
    }

    pub(crate) fn allows_trusted_type_policy_name_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        policy_name: &str,
        is_duplicate: bool,
    ) -> bool {
        let owner = policy_owner_dispatch_scope(scope);
        let Some(snapshot) = self.owner_document_policy_snapshot(owner) else {
            // A context between documents has no policy owner to report
            // against; applying another document's CSP would enforce the
            // wrong policy.
            return true;
        };
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        let check = unsafe { &*self.runtime }.trusted_type_policy_name_csp_check_for_document(
            snapshot.document_handle,
            &snapshot.document_url,
            &snapshot.policy_container.response_content_security_policies,
            &snapshot
                .policy_container
                .response_content_security_report_only_policies,
            &snapshot
                .policy_container
                .content_security_reporting_endpoints,
            policy_name,
            is_duplicate,
        );
        let (mut report_only_violation, mut enforced_violation) = check.into_violations();
        if !self.active_inspector_dispatch
            && let Some((source_file, line_number, column_number)) =
                current_script_violation_location(scope)
        {
            for violation in [&mut enforced_violation, &mut report_only_violation]
                .into_iter()
                .flatten()
            {
                violation.source_file.clone_from(&source_file);
                violation.line_number = line_number;
                violation.column_number = column_number;
            }
        }
        let host_ptr: *mut JsContextHost = self;
        let allowed = enforced_violation.is_none();
        // Policy creation reports expose CSP list ordering. Response policy
        // state is partitioned by disposition, with enforce policies modeled
        // before report-only policies, so preserve that order here.
        if let Some(violation) = enforced_violation {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        if let Some(violation) = report_only_violation {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        allowed
    }

    pub(crate) fn requires_trusted_types_for_script(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> bool {
        self.trusted_types_for_script_requirements(scope)
            .is_enforced()
    }

    pub(crate) fn trusted_types_for_script_requirements(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> TrustedTypesForScriptRequirements {
        self.trusted_types_for_script_requirements_for_owner(policy_owner_dispatch_scope(scope))
            .unwrap_or_default()
    }

    pub(crate) fn allows_trusted_types_eval(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let Some(snapshot) =
            self.owner_document_policy_snapshot(policy_owner_dispatch_scope(scope))
        else {
            return false;
        };
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.allows_trusted_types_eval_for_document(
            snapshot.document_handle,
            &snapshot.policy_container.response_content_security_policies,
            &snapshot
                .policy_container
                .content_security_reporting_endpoints,
        )
    }

    fn child_response_content_security_policies(&self, handle: DomHandle) -> &[String] {
        self.child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.response_content_security_policies())
            .unwrap_or_default()
    }

    fn child_response_content_security_report_only_policies(&self, handle: DomHandle) -> &[String] {
        self.child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.response_content_security_report_only_policies())
            .unwrap_or_default()
    }

    pub(crate) fn dispatch_content_security_policy_violation_event_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        let owner = policy_owner_dispatch_scope(scope);
        self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
            scope, host_ptr, owner, violation,
        );
    }

    pub(crate) fn dispatch_trusted_types_sink_csp_violation_event_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        sink: &str,
        sample: &str,
    ) {
        self.dispatch_trusted_types_sink_csp_violation_event_with_location_best_effort(
            scope, host_ptr, sink, sample, true,
        );
    }

    /// Dispatches a script-execution sink violation from outside regular JS
    /// execution. V8 can expose a current `StackFrame` in this state while its
    /// source-location accessors are invalid, so this path must not probe it.
    pub(crate) fn dispatch_trusted_types_sink_csp_violation_event_without_stack_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        sink: &str,
        sample: &str,
    ) {
        self.dispatch_trusted_types_sink_csp_violation_event_with_location_best_effort(
            scope, host_ptr, sink, sample, false,
        );
    }

    pub(crate) fn dispatch_trusted_types_sink_csp_violation_event_for_owner_without_stack_best_effort<
        's,
    >(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        owner: OwnerDispatchScope,
        sink: &str,
        sample: &str,
    ) {
        for violation in self.trusted_types_sink_csp_violations_for_owner(owner, sink, sample) {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
    }

    fn dispatch_trusted_types_sink_csp_violation_event_with_location_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        sink: &str,
        sample: &str,
        capture_current_script_location: bool,
    ) {
        let source_location = (capture_current_script_location && !self.active_inspector_dispatch)
            .then(|| current_script_violation_location(scope))
            .flatten();
        let owner = policy_owner_dispatch_scope(scope);
        for mut violation in self.trusted_types_sink_csp_violations_for_owner(owner, sink, sample) {
            if let Some((source_file, line_number, column_number)) = &source_location {
                violation.source_file.clone_from(source_file);
                violation.line_number = *line_number;
                violation.column_number = *column_number;
            }
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
    }

    pub(crate) fn dispatch_child_content_security_policy_violation_event_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        if let Err(error) = self
            .dispatch_child_content_security_policy_violation_event(scope, handle, violation, true)
        {
            tracing::error!(
                blocked_uri = violation.blocked_uri.as_str(),
                message = error.to_string().as_str(),
                "child securitypolicyviolation dispatch failed"
            );
        }
    }

    fn dispatch_child_content_security_policy_violation_event_without_report_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        if let Err(error) = self
            .dispatch_child_content_security_policy_violation_event(scope, handle, violation, false)
        {
            tracing::error!(
                blocked_uri = violation.blocked_uri.as_str(),
                message = error.to_string().as_str(),
                "child securitypolicyviolation event-only dispatch failed"
            );
        }
    }

    pub(crate) fn dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort<
        's,
    >(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        identity: crate::native_bridge::WindowDocumentNetworkRequestIdentity,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        if !self.window_document_owner_is_current_for_dispatch_scope(
            identity.owner(),
            identity.dispatch_scope(),
        ) {
            tracing::debug!(
                document_owner = ?identity.owner(),
                dispatch_scope = ?identity.dispatch_scope(),
                blocked_uri = violation.blocked_uri.as_str(),
                "skipped securitypolicyviolation event for retired source document"
            );
            return;
        }
        match identity.dispatch_scope() {
            OwnerDispatchScope::Top => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                unsafe { &mut *self.runtime }
                    .queue_content_security_policy_violation_event_without_report_best_effort(
                        scope, host_ptr, violation,
                    );
            }
            OwnerDispatchScope::Child(handle) => self
                .dispatch_child_content_security_policy_violation_event_without_report_best_effort(
                    scope, handle, violation,
                ),
            OwnerDispatchScope::LightweightPopup(popup_id) => self
                .dispatch_lightweight_popup_content_security_policy_violation_event_without_report_best_effort(
                    scope, popup_id, violation,
                ),
        }
    }

    fn dispatch_content_security_policy_violation_event_for_owner_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        owner: OwnerDispatchScope,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        self.dispatch_content_security_policy_violation_event_for_owner_with_target_best_effort(
            scope, host_ptr, owner, None, violation,
        );
    }

    fn dispatch_content_security_policy_violation_event_for_element_owner_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        owner: OwnerDispatchScope,
        target: DomHandle,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        self.dispatch_content_security_policy_violation_event_for_owner_with_target_best_effort(
            scope,
            host_ptr,
            owner,
            Some(target),
            violation,
        );
    }

    fn dispatch_content_security_policy_violation_event_for_owner_with_target_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        owner: OwnerDispatchScope,
        target: Option<DomHandle>,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        match owner {
            OwnerDispatchScope::Top => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                let runtime = unsafe { &mut *self.runtime };
                if let Some(target) = target {
                    runtime.queue_content_security_policy_violation_event_for_element_best_effort(
                        scope, host_ptr, target, violation,
                    );
                } else {
                    runtime.queue_content_security_policy_violation_event_best_effort(
                        scope, host_ptr, violation,
                    );
                }
            }
            OwnerDispatchScope::Child(handle) => {
                // Child documents share this renderer PageVM and its Audits
                // issue storage even though their DOM event targets differ.
                unsafe { &mut *self.runtime }
                    .record_content_security_policy_inspector_issue(target, violation);
                self.dispatch_child_content_security_policy_violation_event_best_effort(
                    scope, handle, violation,
                );
            }
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                // A lightweight popup shares this renderer owner only until it
                // is projected as its own DevTools target. Publishing through
                // the opener PageVM would leak the popup issue to the wrong
                // target; popup-owned Audits delivery needs an explicit owner
                // handoff alongside popup activation.
                self.dispatch_lightweight_popup_content_security_policy_violation_event_best_effort(
                    scope, popup_id, violation,
                );
            }
        }
    }

    pub(crate) fn dispatch_frame_navigation_csp_violation_event_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        frame_handle: DomHandle,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        match self.owner_dispatch_scope_for_node(frame_handle) {
            Some(OwnerDispatchScope::Child(source_child)) => {
                self.dispatch_child_content_security_policy_violation_event_best_effort(
                    scope,
                    source_child,
                    violation,
                );
            }
            Some(OwnerDispatchScope::Top) => {
                let host_ptr: *mut JsContextHost = self;
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                unsafe { &mut *self.runtime }
                    .queue_content_security_policy_violation_event_best_effort(
                        scope, host_ptr, violation,
                    );
            }
            Some(OwnerDispatchScope::LightweightPopup(popup_id)) => {
                self.dispatch_lightweight_popup_content_security_policy_violation_event_best_effort(
                    scope, popup_id, violation,
                );
            }
            None => {
                tracing::error!(
                    blocked_uri = violation.blocked_uri.as_str(),
                    "frame navigation CSP violation had no source document"
                );
            }
        }
    }

    fn dispatch_child_content_security_policy_violation_event<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        violation: &DocumentContentSecurityPolicyViolation,
        send_report: bool,
    ) -> anyhow::Result<()> {
        let window = self
            .child_browsing_context_window_wrapper(scope, handle)
            .ok_or_else(|| anyhow::anyhow!("child window wrapper is unavailable"))?;
        let event = create_content_security_policy_violation_event(
            scope,
            window.into(),
            window.into(),
            violation,
        )?;
        if send_report {
            let fields = ContentSecurityPolicyViolationEventFields::from(violation);
            let document_owner =
                self.current_child_document_task_owner(handle)
                    .ok_or_else(|| {
                        anyhow::anyhow!("child CSP violation document owner is unavailable")
                    })?;
            crate::network_host::send_content_security_policy_reports_for_window(
                scope,
                self,
                document_owner,
                Some(handle),
                &fields,
                &violation.report_uri_endpoints,
                &violation.report_to_endpoints,
            );
        }
        self.dispatch_child_window_event(scope, handle, "securitypolicyviolation", event);
        Ok(())
    }
}

fn current_script_violation_location(
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
