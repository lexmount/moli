use super::{JsContextHost, OwnerDispatchScope, WindowExecutionContextIdentity};
use crate::{
    content_security_policy::{
        ContentSecurityPolicyNonUrlKind, ContentSecurityPolicyRedirectStatus,
        ContentSecurityPolicyReportingEndpoints, ContentSecurityPolicyScriptElementRequest,
        ContentSecurityPolicyViolationEventFields, TrustedTypesForScriptRequirements,
    },
    context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
    document_runtime::{
        DocumentContentSecurityPolicyCheck, DocumentContentSecurityPolicyViolation,
        DocumentSubresourceCspKind, DomHandle, create_content_security_policy_violation_event,
    },
    dom::native::Node,
    native_bridge::{active_child_window_handle, child_window_handle_from_marker_data},
    util::get_private_value,
};

#[derive(Debug)]
#[must_use = "the caller must stop a request when CSP blocks it"]
pub(crate) enum DocumentCspOutcome {
    Allowed,
    Blocked(DocumentContentSecurityPolicyViolation),
    SkippedChildContext,
}

fn policy_child_window_handle(scope: &mut v8::PinScope<'_, '_>) -> Option<DomHandle> {
    if let Some(handle) = active_child_window_handle(scope) {
        return Some(handle);
    }
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| child_window_handle_from_marker_data(scope, value))
}

impl DocumentCspOutcome {
    pub(crate) fn blocks_request(&self) -> bool {
        matches!(self, Self::Blocked(_))
    }

    pub(crate) fn into_blocking_violation(self) -> Option<DocumentContentSecurityPolicyViolation> {
        match self {
            Self::Blocked(violation) => Some(violation),
            Self::Allowed | Self::SkippedChildContext => None,
        }
    }
}

impl JsContextHost {
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
        let policy = self.document_policy_container_snapshot_for_owner(owner)?;
        Some(crate::document_runtime::DocumentConnectPolicySnapshot::from_policy_container(&policy))
    }

    pub(crate) fn document_policy_container_snapshot_for_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<crate::document_runtime::DocumentPolicyContainer> {
        Some(match owner {
            OwnerDispatchScope::Top => self.document_policy_container().clone(),
            OwnerDispatchScope::Child(handle) => {
                self.child_browsing_context_policy_container_snapshot(handle)?
            }
        })
    }

    pub(crate) fn document_policy_container_snapshot_for_identity(
        &self,
        identity: WindowExecutionContextIdentity,
    ) -> Option<crate::document_runtime::DocumentPolicyContainer> {
        self.window_execution_context_identity_is_current(identity)
            .then(|| identity.dispatch_scope())
            .and_then(|owner| self.document_policy_container_snapshot_for_owner(owner))
    }

    /// Reads the active policy of the Document that owns `node`.
    ///
    /// Form submission has an early preparation gate and a second gate after
    /// entry-list construction. Both must consult the current source
    /// Document; target lookup and new-context admission cannot reconstruct
    /// this permission, even if direct-submit target preselection eventually
    /// moves ahead of the late gate.
    pub(crate) fn document_allows_form_submission_for_node(&self, node: DomHandle) -> bool {
        self.owner_dispatch_scope_for_node(node)
            .and_then(|owner| self.document_policy_container_snapshot_for_owner(owner))
            .is_none_or(|policy| policy.sandbox.allows_forms)
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
        if policy_child_window_handle(scope).is_some() {
            return DocumentCspOutcome::SkippedChildContext;
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

    pub(crate) fn allows_form_action_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        request_url: &url::Url,
    ) -> bool {
        let Some(check) = self.form_action_csp_check_for_owner(owner, request_url) else {
            // A navigation swap can temporarily leave a compatibility owner
            // without a committed policy container. Do not substitute the
            // root Document's form-action policy for that missing owner.
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

    fn form_action_csp_check_for_owner(
        &self,
        owner: OwnerDispatchScope,
        request_url: &url::Url,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        match owner {
            OwnerDispatchScope::Top => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                Some(unsafe { &*self.runtime }.form_action_csp_check(request_url))
            }
            OwnerDispatchScope::Child(handle) => {
                let document_url = self.child_browsing_context_current_url(handle)?;
                let response_policies =
                    self.child_effective_response_content_security_policies(handle);
                let response_report_only_policies =
                    self.child_effective_response_content_security_report_only_policies(handle);
                let response_reporting_endpoints =
                    self.child_effective_content_security_reporting_endpoints(handle);
                Some(
                    unsafe { &*self.runtime }.form_action_csp_check_for_child_document(
                        self.child_browsing_context_document_handle(handle),
                        &document_url,
                        &response_policies,
                        &response_report_only_policies,
                        &response_reporting_endpoints,
                        request_url,
                    ),
                )
            }
        }
    }

    fn allows_inline_source_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        kind: ContentSecurityPolicyNonUrlKind,
        source: &str,
    ) -> bool {
        let Some(check) = self.inline_source_csp_check_for_owner(scope, owner, kind, source) else {
            // Deliberately fail open only while the exact document owner has
            // no active URL/policy context (for example during a navigation
            // swap). CSP is document-scoped: applying another document's
            // policy here would enforce the wrong owner. Committed documents
            // must always produce a check.
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
        _scope: &mut v8::PinScope<'_, '_>,
        owner: OwnerDispatchScope,
        kind: ContentSecurityPolicyNonUrlKind,
        source: &str,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        match owner {
            OwnerDispatchScope::Top => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                Some(unsafe { &*self.runtime }.inline_source_csp_check(kind, source))
            }
            OwnerDispatchScope::Child(handle) => {
                let document_url = self.child_browsing_context_current_url(handle)?;
                let response_policies =
                    self.child_effective_response_content_security_policies(handle);
                let response_report_only_policies =
                    self.child_effective_response_content_security_report_only_policies(handle);
                let response_reporting_endpoints =
                    self.child_effective_content_security_reporting_endpoints(handle);
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                Some(
                    unsafe { &*self.runtime }.inline_source_csp_check_for_child_document(
                        self.child_browsing_context_document_handle(handle),
                        &document_url,
                        &response_policies,
                        &response_report_only_policies,
                        &response_reporting_endpoints,
                        kind,
                        source,
                    ),
                )
            }
        }
    }

    fn inline_script_element_csp_check_for_owner(
        &self,
        owner: OwnerDispatchScope,
        source: &str,
        request: ContentSecurityPolicyScriptElementRequest<'_>,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        match owner {
            OwnerDispatchScope::Top => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                Some(unsafe { &*self.runtime }.inline_script_element_csp_check(source, request))
            }
            OwnerDispatchScope::Child(handle) => {
                let document_url = self.child_browsing_context_current_url(handle)?;
                let response_policies =
                    self.child_effective_response_content_security_policies(handle);
                let response_report_only_policies =
                    self.child_effective_response_content_security_report_only_policies(handle);
                let response_reporting_endpoints =
                    self.child_effective_content_security_reporting_endpoints(handle);
                Some(
                    unsafe { &*self.runtime }.inline_script_element_csp_check_for_child_document(
                        self.child_browsing_context_document_handle(handle),
                        &document_url,
                        &response_policies,
                        &response_report_only_policies,
                        &response_reporting_endpoints,
                        source,
                        request,
                    ),
                )
            }
        }
    }

    /// Returns the exact owner registered to the currently executing V8 realm.
    ///
    /// This deliberately does not infer an entry/incumbent realm. Web APIs that
    /// need either identity must capture that concrete V8 Context themselves.
    pub(crate) fn current_realm_owner_dispatch_scope(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<OwnerDispatchScope> {
        self.current_runtime_window_execution_context_identity(scope)
            .map(|identity| identity.dispatch_scope())
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
        let response_reporting_endpoints = self.child_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }
            .document_connect_csp_check_for_child_document_with_redirect_status(
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
        match self.frame_navigation_source_child_handle(frame_handle)? {
            Some(source_child) => {
                let document_url = self.child_browsing_context_current_url(source_child)?;
                let response_policies =
                    self.child_effective_response_content_security_policies(source_child);
                let response_reporting_endpoints =
                    self.child_effective_content_security_reporting_endpoints(source_child);
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                unsafe { &*self.runtime }.document_frame_csp_violation_for_child_document(
                    self.child_browsing_context_document_handle(source_child),
                    &document_url,
                    &response_policies,
                    &response_reporting_endpoints,
                    request_url,
                )
            }
            None => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                unsafe { &*self.runtime }.document_frame_csp_violation(request_url)
            }
        }
    }

    pub(crate) fn frame_navigation_csp_report_only_violation(
        &self,
        frame_handle: DomHandle,
        request_url: &url::Url,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        match self.frame_navigation_source_child_handle(frame_handle)? {
            Some(source_child) => {
                let document_url = self.child_browsing_context_current_url(source_child)?;
                let response_report_only_policies = self
                    .child_effective_response_content_security_report_only_policies(source_child);
                let response_reporting_endpoints =
                    self.child_effective_content_security_reporting_endpoints(source_child);
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                unsafe { &*self.runtime }
                    .document_frame_csp_report_only_violation_for_child_document(
                        &document_url,
                        &response_report_only_policies,
                        &response_reporting_endpoints,
                        request_url,
                    )
            }
            None => {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                unsafe { &*self.runtime }.document_frame_csp_report_only_violation(request_url)
            }
        }
    }

    fn frame_navigation_source_child_handle(
        &self,
        frame_handle: DomHandle,
    ) -> Option<Option<DomHandle>> {
        let owner_document = self
            .dom_host()
            .node(frame_handle)
            .and_then(Node::owner_document)?;
        if owner_document == self.document_handle() {
            return Some(None);
        }
        self.child_browsing_context_host_for_document_handle(owner_document)
            .map(Some)
    }

    fn child_effective_response_content_security_policies(
        &self,
        child_handle: DomHandle,
    ) -> Vec<String> {
        let mut policies = Vec::new();
        if self
            .child_browsing_contexts
            .get(&child_handle)
            .is_some_and(|entry| entry.security_origin_inherited())
        {
            policies.extend(self.parent_effective_response_content_security_policies(child_handle));
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
            .is_some_and(|entry| entry.security_origin_inherited())
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

    fn parent_effective_response_content_security_policies(
        &self,
        child_handle: DomHandle,
    ) -> Vec<String> {
        match self.child_browsing_context_parent_handle(child_handle) {
            Some(parent) => self.child_effective_response_content_security_policies(parent),
            None => unsafe { &*self.runtime }
                .response_content_security_policies()
                .to_vec(),
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
            .is_some_and(|entry| entry.security_origin_inherited());
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
    ) -> bool {
        let (report_only_violation, enforced_violation) =
            if let Some(handle) = policy_child_window_handle(scope) {
                (
                    self.child_eval_csp_report_only_violation(handle, allow_trusted_types_eval),
                    self.child_eval_csp_violation(handle, allow_trusted_types_eval),
                )
            } else {
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                let runtime = unsafe { &*self.runtime };
                (
                    runtime.eval_csp_report_only_violation(allow_trusted_types_eval),
                    runtime.eval_csp_violation(allow_trusted_types_eval),
                )
            };
        self.apply_code_generation_csp_check(scope, report_only_violation, enforced_violation, true)
    }

    pub(crate) fn allows_wasm_code_generation_by_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> bool {
        let report_only_violation = self.wasm_eval_csp_report_only_violation(scope);
        let enforced_violation = self.wasm_eval_csp_violation(scope);
        self.apply_code_generation_csp_check(
            scope,
            report_only_violation,
            enforced_violation,
            false,
        )
    }

    fn apply_code_generation_csp_check<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        mut report_only_violation: Option<DocumentContentSecurityPolicyViolation>,
        mut enforced_violation: Option<DocumentContentSecurityPolicyViolation>,
        include_call_location: bool,
    ) -> bool {
        if report_only_violation.is_none() && enforced_violation.is_none() {
            return true;
        }
        if include_call_location {
            let (line_number, column_number) = current_script_call_location(scope);
            for violation in [&mut report_only_violation, &mut enforced_violation]
                .into_iter()
                .flatten()
            {
                violation.line_number = line_number;
                violation.column_number = column_number;
            }
        }
        let host_ptr: *mut JsContextHost = self;
        if let Some(violation) = report_only_violation {
            self.dispatch_content_security_policy_violation_event_best_effort(
                scope, host_ptr, &violation,
            );
        }
        let Some(violation) = enforced_violation else {
            return true;
        };
        self.dispatch_content_security_policy_violation_event_best_effort(
            scope, host_ptr, &violation,
        );
        false
    }

    pub(crate) fn wasm_eval_csp_violation(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        if let Some(handle) = policy_child_window_handle(scope) {
            return self.child_wasm_eval_csp_violation(handle);
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.wasm_eval_csp_violation()
    }

    pub(crate) fn wasm_eval_csp_report_only_violation(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        if let Some(handle) = policy_child_window_handle(scope) {
            return self.child_wasm_eval_csp_report_only_violation(handle);
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.wasm_eval_csp_report_only_violation()
    }

    pub(crate) fn child_wasm_eval_csp_violation(
        &self,
        handle: DomHandle,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.child_browsing_context_current_url(handle)?;
        let response_policies = self.child_response_content_security_policies(handle);
        let response_reporting_endpoints = self.child_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.wasm_eval_csp_violation_for_child_document(
            self.child_browsing_context_document_handle(handle),
            &document_url,
            response_policies,
            &response_reporting_endpoints,
        )
    }

    pub(crate) fn child_wasm_eval_csp_report_only_violation(
        &self,
        handle: DomHandle,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.child_browsing_context_current_url(handle)?;
        let response_report_only_policies =
            self.child_response_content_security_report_only_policies(handle);
        let response_reporting_endpoints = self.child_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.wasm_eval_csp_report_only_violation_for_child_document(
            &document_url,
            response_report_only_policies,
            &response_reporting_endpoints,
        )
    }

    fn child_eval_csp_violation(
        &self,
        handle: DomHandle,
        allow_trusted_types_eval: bool,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.child_browsing_context_current_url(handle)?;
        let response_policies = self.child_response_content_security_policies(handle);
        let response_reporting_endpoints = self.child_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.eval_csp_violation_for_child_document(
            self.child_browsing_context_document_handle(handle),
            &document_url,
            response_policies,
            &response_reporting_endpoints,
            allow_trusted_types_eval,
        )
    }

    fn child_eval_csp_report_only_violation(
        &self,
        handle: DomHandle,
        allow_trusted_types_eval: bool,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.child_browsing_context_current_url(handle)?;
        let response_report_only_policies =
            self.child_response_content_security_report_only_policies(handle);
        let response_reporting_endpoints = self.child_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.eval_csp_report_only_violation_for_child_document(
            &document_url,
            response_report_only_policies,
            &response_reporting_endpoints,
            allow_trusted_types_eval,
        )
    }

    pub(crate) fn trusted_types_sink_csp_violation(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        sink: &str,
        sample: &str,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        if let Some(handle) = policy_child_window_handle(scope) {
            return self.child_trusted_types_sink_csp_violation(handle, sink, sample);
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.trusted_types_sink_csp_violation(sink, sample)
    }

    pub(crate) fn trusted_types_sink_csp_report_only_violation(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        sink: &str,
        sample: &str,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        if let Some(handle) = policy_child_window_handle(scope) {
            return self.child_trusted_types_sink_csp_report_only_violation(handle, sink, sample);
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.trusted_types_sink_csp_report_only_violation(sink, sample)
    }

    pub(crate) fn child_trusted_types_sink_csp_violation(
        &self,
        handle: DomHandle,
        sink: &str,
        sample: &str,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.child_browsing_context_current_url(handle)?;
        let response_policies = self.child_effective_response_content_security_policies(handle);
        let response_reporting_endpoints =
            self.child_effective_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.trusted_types_sink_csp_violation_for_child_document(
            self.child_browsing_context_document_handle(handle),
            &document_url,
            &response_policies,
            &response_reporting_endpoints,
            sink,
            sample,
        )
    }

    pub(crate) fn child_trusted_types_sink_csp_report_only_violation(
        &self,
        handle: DomHandle,
        sink: &str,
        sample: &str,
    ) -> Option<DocumentContentSecurityPolicyViolation> {
        let document_url = self.child_browsing_context_current_url(handle)?;
        let response_report_only_policies =
            self.child_effective_response_content_security_report_only_policies(handle);
        let response_reporting_endpoints =
            self.child_effective_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.trusted_types_sink_csp_report_only_violation_for_child_document(
            &document_url,
            &response_report_only_policies,
            &response_reporting_endpoints,
            sink,
            sample,
        )
    }

    pub(crate) fn allows_trusted_type_policy_name(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        policy_name: &str,
    ) -> bool {
        if let Some(handle) = policy_child_window_handle(scope) {
            return self.child_allows_trusted_type_policy_name(handle, policy_name);
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.allows_trusted_type_policy_name(policy_name)
    }

    pub(crate) fn requires_trusted_types_for_script(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> bool {
        if let Some(handle) = policy_child_window_handle(scope) {
            return self.child_requires_trusted_types_for_script(handle);
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.requires_trusted_types_for_script()
    }

    pub(crate) fn trusted_types_for_script_requirements(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> TrustedTypesForScriptRequirements {
        if let Some(handle) = policy_child_window_handle(scope) {
            let response_policies = self.child_effective_response_content_security_policies(handle);
            let response_report_only_policies =
                self.child_effective_response_content_security_report_only_policies(handle);
            let response_reporting_endpoints =
                self.child_effective_content_security_reporting_endpoints(handle);
            // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
            return unsafe { &*self.runtime }
                .trusted_types_for_script_requirements_for_child_document(
                    self.child_browsing_context_document_handle(handle),
                    &response_policies,
                    &response_report_only_policies,
                    &response_reporting_endpoints,
                );
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.trusted_types_for_script_requirements()
    }

    pub(crate) fn allows_trusted_types_eval(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        if let Some(handle) = policy_child_window_handle(scope) {
            let response_policies = self.child_response_content_security_policies(handle);
            let response_reporting_endpoints =
                self.child_content_security_reporting_endpoints(handle);
            // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
            return unsafe { &*self.runtime }.allows_trusted_types_eval_for_child_document(
                self.child_browsing_context_document_handle(handle),
                response_policies,
                &response_reporting_endpoints,
            );
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.allows_trusted_types_eval()
    }

    pub(crate) fn child_allows_trusted_type_policy_name(
        &self,
        handle: DomHandle,
        policy_name: &str,
    ) -> bool {
        let response_policies = self.child_response_content_security_policies(handle);
        let response_reporting_endpoints = self.child_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.allows_trusted_type_policy_name_for_child_document(
            self.child_browsing_context_document_handle(handle),
            response_policies,
            &response_reporting_endpoints,
            policy_name,
        )
    }

    pub(crate) fn child_requires_trusted_types_for_script(&self, handle: DomHandle) -> bool {
        let response_policies = self.child_response_content_security_policies(handle);
        let response_reporting_endpoints = self.child_content_security_reporting_endpoints(handle);
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &*self.runtime }.requires_trusted_types_for_script_for_child_document(
            self.child_browsing_context_document_handle(handle),
            response_policies,
            &response_reporting_endpoints,
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

    fn child_content_security_reporting_endpoints(
        &self,
        handle: DomHandle,
    ) -> ContentSecurityPolicyReportingEndpoints {
        self.child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.content_security_reporting_endpoints())
            .unwrap_or_default()
    }

    pub(crate) fn dispatch_content_security_policy_violation_event_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        if let Some(handle) = policy_child_window_handle(scope) {
            self.dispatch_child_content_security_policy_violation_event_best_effort(
                scope, handle, violation,
            );
            return;
        }
        // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
        unsafe { &mut *self.runtime }
            .queue_content_security_policy_violation_event_best_effort(scope, host_ptr, violation);
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

    fn dispatch_trusted_types_sink_csp_violation_event_with_location_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        sink: &str,
        sample: &str,
        capture_current_script_location: bool,
    ) {
        let source_location = (capture_current_script_location && !self.active_inspector_dispatch)
            .then(|| v8::StackTrace::current_stack_trace(scope, 1))
            .flatten()
            .and_then(|stack| stack.get_frame(scope, 0))
            .map(|frame| {
                let source_file = frame
                    .get_script_name_or_source_url(scope)
                    .map(|source| source.to_rust_string_lossy(scope))
                    .map(|source| {
                        crate::content_security_policy::content_security_policy_source_file_for_report(
                            &source,
                        )
                    })
                    .unwrap_or_default();
                let line_number = i32::try_from(frame.get_line_number())
                    .unwrap_or_default()
                    .max(0);
                let column_number =
                    i32::try_from(frame.get_column()).unwrap_or_default().max(0);
                (source_file, line_number, column_number)
            });
        let violations = [
            self.trusted_types_sink_csp_report_only_violation(scope, sink, sample),
            self.trusted_types_sink_csp_violation(scope, sink, sample),
        ];
        for mut violation in violations.into_iter().flatten() {
            if let Some((source_file, line_number, column_number)) = &source_location {
                violation.source_file.clone_from(source_file);
                violation.line_number = *line_number;
                violation.column_number = *column_number;
            }
            self.dispatch_content_security_policy_violation_event_best_effort(
                scope, host_ptr, &violation,
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
        }
    }

    pub(crate) fn dispatch_frame_navigation_csp_violation_event_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        frame_handle: DomHandle,
        violation: &DocumentContentSecurityPolicyViolation,
    ) {
        match self.frame_navigation_source_child_handle(frame_handle) {
            Some(Some(source_child)) => {
                self.dispatch_child_content_security_policy_violation_event_best_effort(
                    scope,
                    source_child,
                    violation,
                );
            }
            Some(None) => {
                let host_ptr: *mut JsContextHost = self;
                // SAFETY: JsContextHost is owned by the ScriptVm that owns this DocumentRuntime.
                unsafe { &mut *self.runtime }
                    .queue_content_security_policy_violation_event_best_effort(
                        scope, host_ptr, violation,
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

fn current_script_call_location(scope: &v8::PinScope<'_, '_>) -> (i32, i32) {
    let Some(stack) = v8::StackTrace::current_stack_trace(scope, 1) else {
        return (0, 0);
    };
    let Some(frame) = stack.get_frame(scope, 0) else {
        return (0, 0);
    };
    let line = i32::try_from(frame.get_line_number()).unwrap_or(0).max(0);
    let column = i32::try_from(frame.get_column()).unwrap_or(0).max(0);
    (line, column)
}
