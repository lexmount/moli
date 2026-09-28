use super::{JsContextHost, OwnerDispatchScope};
use crate::{
    content_security_policy::{
        ContentSecurityPolicyDisposition, ContentSecurityPolicyNonUrlKind,
        ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyReportingEndpoints,
        ContentSecurityPolicyScriptElementRequest, ContentSecurityPolicyViolationEventFields,
        TrustedTypesForScriptRequirements, current_script_violation_location,
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
use url::Url;

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

fn security_origin_from_serialized(origin: &str) -> url::Origin {
    url::Url::parse(origin).map_or_else(|_| url::Origin::new_opaque(), |url| url.origin())
}

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

fn policy_owner_dispatch_scope_for_global<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> OwnerDispatchScope {
    if let Some(handle) = get_private_value(scope, global, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| child_window_handle_from_marker_data(scope, value))
    {
        return OwnerDispatchScope::Child(handle);
    }
    if let Some(id) = crate::native_bridge::lightweight_popup_id_from_window(scope, global) {
        return OwnerDispatchScope::LightweightPopup(id);
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
    pub(crate) fn document_allows_autofocus(&self, document: DomHandle) -> bool {
        let Some(endpoint) = self.window_endpoint_for_document(document) else {
            return false;
        };
        let dispatch_scope = endpoint.dispatch_scope();
        let allows = |policy: &DocumentPolicyContainer| {
            policy.sandbox.allows_scripts
                && (policy
                    .permissions_policy
                    .focus_without_user_activation_enabled()
                    || self.window_has_transient_user_activation(dispatch_scope))
        };
        match dispatch_scope {
            OwnerDispatchScope::Top => allows(self.document_policy_container()),
            OwnerDispatchScope::LightweightPopup(popup_id) => self
                .lightweight_popup_policy_container(popup_id)
                .is_some_and(allows),
            OwnerDispatchScope::Child(handle) => self
                .child_browsing_contexts
                .get(&handle)
                .is_some_and(|entry| allows(entry.document_policy_container())),
        }
    }

    pub(in crate::native_bridge::context_host) fn preload_link_csp_check(
        &self,
        link: DomHandle,
        request_url: &url::Url,
    ) -> Option<DocumentContentSecurityPolicyCheck> {
        let owner = self.owner_dispatch_scope_for_node(link)?;
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        Some(
            unsafe { &*self.runtime }.preload_link_csp_check_for_document(
                snapshot.document_handle,
                &snapshot.document_url,
                &snapshot.policy_container,
                link,
                request_url,
            ),
        )
    }

    pub(crate) fn base_url_content_security_policy_check(
        &self,
        check: &crate::dom::native::DocumentBaseUrlPolicyCheck,
    ) -> Option<(
        crate::frame_owner_model::FrameDocumentTaskOwner,
        DocumentContentSecurityPolicyCheck,
    )> {
        if unsafe { &*self.runtime }.bypass_content_security_policy() {
            return None;
        }
        let dispatch_scope = self.owner_dispatch_scope_for_node(check.document)?;
        let owner = match dispatch_scope {
            OwnerDispatchScope::Top => self.current_main_document_task_owner()?,
            OwnerDispatchScope::Child(handle) => self.current_child_document_task_owner(handle)?,
            OwnerDispatchScope::LightweightPopup(_) => return None,
        };
        let snapshot = self.owner_document_policy_snapshot(dispatch_scope)?;
        let result = unsafe { &*self.runtime }.base_url_content_security_policy_check_for_document(
            check.document,
            &snapshot.document_url,
            &snapshot.policy_container,
            &check.url,
        );
        Some((owner, result))
    }

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
                let policy_container =
                    self.child_browsing_context_policy_container_snapshot(handle)?;
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

    pub(crate) fn document_policy_container_for_inheritance(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<DocumentPolicyContainer> {
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        let mut policy = snapshot.policy_container;
        policy
            .content_security_policy_self_url
            .get_or_insert(snapshot.document_url);
        if let Some(document) = snapshot.document_handle {
            // Meta policies are delivered separately from response headers.
            // Capture their current values so later creator mutations cannot
            // change the new Document's policy container.
            policy.referrer_policy =
                crate::context_bootstrap::document_referrer_policy_for_native_document(
                    self, document,
                );
            policy.inherited_meta_content_security_policies = unsafe { &*self.runtime }
                .meta_content_security_policy_strings_for_document(document);
        }
        Some(policy)
    }

    pub(crate) fn capture_about_document_state(
        &self,
        owner: OwnerDispatchScope,
        destination: &Url,
        source_element: Option<DomHandle>,
    ) -> Option<crate::runtime::RendererAboutDocumentState> {
        if !moli_url::is_about_blank(destination) {
            return None;
        }
        let owner = source_element
            .and_then(|element| self.owner_dispatch_scope_for_node(element))
            .unwrap_or(owner);
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        let origin = self.window_access_origin_for_dispatch_scope(owner)?;
        let mut policy_container = self.document_policy_container_for_inheritance(owner)?;
        let element_referrer_policy = source_element
            .and_then(|handle| self.dom_host().node(handle))
            .and_then(|node| node.as_element())
            .and_then(|element| {
                if element.attribute("rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("noreferrer"))
                }) {
                    Some("no-referrer".to_owned())
                } else {
                    element
                        .attribute("referrerpolicy")
                        .and_then(crate::referrer_policy::normalize_referrer_policy)
                }
            });
        policy_container.document_referrer = if origin.serialized_origin() == "null" {
            String::new()
        } else {
            moli_fetch::referrer_value(
                &snapshot.document_url,
                destination,
                element_referrer_policy.as_deref(),
                policy_container.referrer_policy.as_deref(),
            )
            .unwrap_or_default()
        };
        let base_url = snapshot
            .document_handle
            .map(|document| self.document_base_url_for_handle(document))
            .unwrap_or(snapshot.document_url);
        Some(crate::runtime::RendererAboutDocumentState::new(
            origin,
            base_url,
            policy_container,
        ))
    }

    pub(crate) fn current_about_document_state(
        &self,
    ) -> Option<crate::runtime::RendererAboutDocumentState> {
        let state = self.about_document_state.as_ref()?;
        let mut policy = self.document_policy_container_for_inheritance(OwnerDispatchScope::Top)?;
        // Reload/traversal retains the navigation's original policy, unlike
        // a newly created Document inheriting the creator's current meta value.
        policy.referrer_policy = self.response_referrer_policy().map(ToOwned::to_owned);
        Some(crate::runtime::RendererAboutDocumentState::new(
            self.window_access_origin_for_dispatch_scope(OwnerDispatchScope::Top)?,
            state.base_url().clone(),
            policy,
        ))
    }

    pub(crate) fn set_main_about_document_state(
        &mut self,
        state: Option<crate::runtime::RendererAboutDocumentState>,
    ) {
        let state = state.filter(|_| moli_url::is_about_blank(self.document_url()));
        let document = self.document_handle();
        self.dom_host_mut()
            .set_document_fallback_base_url_for_handle(
                document,
                state.as_ref().map(|state| state.base_url().clone()),
            );
        if let Some(super::WindowAccessOrigin::Tuple {
            document_domain: Some(domain),
            ..
        }) = state.as_ref().map(|state| state.origin())
        {
            self.document_domain_override.set(domain.clone());
        }
        self.about_document_state = state;
    }

    pub(crate) fn document_connect_policy_snapshot_for_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<crate::document_runtime::DocumentConnectPolicySnapshot> {
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        // SAFETY: JsContextHost belongs to the ScriptVm that owns this runtime.
        Some(
            unsafe { &*self.runtime }.document_connect_policy_snapshot_for_document(
                snapshot.document_handle,
                &snapshot.policy_container,
            ),
        )
    }

    pub(crate) fn local_worker_content_security_policy_source_for_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<crate::content_security_policy::ContentSecurityPolicySource> {
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        // SAFETY: this host and its DocumentRuntime belong to the same ScriptVm.
        Some(
            unsafe { &*self.runtime }.local_worker_content_security_policy_source(
                snapshot.document_handle,
                &snapshot.document_url,
                &snapshot.policy_container,
            ),
        )
    }

    pub(crate) fn local_worker_content_security_policy_source_for_global<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
    ) -> Option<crate::content_security_policy::ContentSecurityPolicySource> {
        self.local_worker_content_security_policy_source_for_owner(
            policy_owner_dispatch_scope_for_global(scope, global),
        )
    }

    pub(crate) fn object_url_policy_source_for_global<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
    ) -> Option<crate::blob::ObjectUrlPolicySource> {
        let owner = policy_owner_dispatch_scope_for_global(scope, global);
        let snapshot = self.owner_document_policy_snapshot(owner)?;
        Some(crate::blob::ObjectUrlPolicySource {
            content_security_policy: self
                .local_worker_content_security_policy_source_for_owner(owner)?,
            referrer_policy: snapshot.policy_container.referrer_policy,
            document_meta_referrer_policy: snapshot
                .document_handle
                .and_then(|document| self.dom_host().node(document))
                .and_then(moli_dom::native::Node::as_document)
                .map(moli_dom::native::Document::meta_referrer_policy_source),
        })
    }

    pub(crate) fn document_permissions_policy_for_owner(
        &self,
        owner: OwnerDispatchScope,
    ) -> Option<crate::permissions_policy::DocumentPermissionsPolicy> {
        match owner {
            OwnerDispatchScope::Top => {
                Some(self.document_policy_container().permissions_policy.clone())
            }
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
                .map(|policy| policy.permissions_policy.clone()),
        }
    }

    pub(crate) fn document_permissions_policy_for_document_handle(
        &self,
        document: DomHandle,
    ) -> Option<crate::permissions_policy::DocumentPermissionsPolicy> {
        if document == self.document_handle() {
            return Some(self.document_policy_container().permissions_policy.clone());
        }
        if let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document) {
            return self
                .lightweight_popup_policy_container(popup_id)
                .map(|policy| policy.permissions_policy.clone());
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

    // Read the committed owner's security origin, including inherited blank
    // documents and sandboxing. A pending navigation's URL is not authority.
    pub(crate) fn document_security_origin(&self, document: DomHandle) -> url::Origin {
        if document == self.document_handle() && self.document_sandbox_policy().forces_opaque_origin
        {
            return url::Origin::new_opaque();
        }
        if let Some(child) = self.child_browsing_context_host_for_document_handle(document) {
            return self
                .child_document_security_origin(child)
                .unwrap_or_else(url::Origin::new_opaque);
        }
        let serialized = (document == self.document_handle())
            .then(|| {
                self.frame_owner_store
                    .current_main_owner_snapshot()
                    .map(|owner| owner.settings.origin)
            })
            .flatten()
            .or_else(|| {
                self.lightweight_popup_id_for_document_handle(document)
                    .and_then(|popup| self.lightweight_popup_origin(popup))
            });
        serialized.map_or_else(
            || self.document_url_for_handle(document).origin(),
            |origin| security_origin_from_serialized(&origin),
        )
    }

    // Commit observers run before the adapter updates its Document-handle map.
    pub(crate) fn child_document_security_origin(&self, child: DomHandle) -> Option<url::Origin> {
        let owner = self.frame_owner_store.current_child_owner_snapshot(child)?;
        Some(
            if owner
                .settings
                .document_policy_container
                .sandbox
                .forces_opaque_origin
            {
                url::Origin::new_opaque()
            } else {
                security_origin_from_serialized(&owner.settings.origin)
            },
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

    pub(crate) fn trusted_types_for_script_requirements_for_global<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
    ) -> TrustedTypesForScriptRequirements {
        self.trusted_types_for_script_requirements_for_owner(
            policy_owner_dispatch_scope_for_global(scope, global),
        )
        .unwrap_or_default()
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
        let (report_only_violations, enforced_violations) = self
            .document_subresource_csp_check(request_url, kind)
            .into_violations();
        let host_ptr: *mut JsContextHost = self;
        for violation in report_only_violations {
            self.dispatch_content_security_policy_violation_event_best_effort(
                scope, host_ptr, &violation,
            );
        }
        for violation in &enforced_violations {
            self.dispatch_content_security_policy_violation_event_best_effort(
                scope, host_ptr, violation,
            );
        }
        match enforced_violations.into_iter().next() {
            Some(violation) => DocumentCspOutcome::Blocked(violation),
            None => DocumentCspOutcome::Allowed,
        }
    }

    pub(crate) fn check_element_subresource_csp<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        element: DomHandle,
        request_url: &url::Url,
        kind: DocumentSubresourceCspKind,
    ) -> DocumentCspOutcome {
        let Some(owner) = self.owner_dispatch_scope_for_node(element) else {
            return DocumentCspOutcome::Allowed;
        };
        let Some(snapshot) = self.owner_document_policy_snapshot(owner) else {
            return DocumentCspOutcome::Allowed;
        };
        // The element's Document owns this request even when a different
        // Window's script changes its source or queues the update microtask.
        // SAFETY: this host and its DocumentRuntime are owned by the same ScriptVm.
        let (report_only_violations, enforced_violations) = unsafe { &*self.runtime }
            .document_subresource_csp_check_for_document(
                snapshot.document_handle,
                &snapshot.document_url,
                &snapshot.policy_container,
                request_url,
                kind,
            )
            .into_violations();
        let host_ptr: *mut JsContextHost = self;
        for violation in report_only_violations {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        for violation in &enforced_violations {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, violation,
            );
        }
        match enforced_violations.into_iter().next() {
            Some(violation) => DocumentCspOutcome::Blocked(violation),
            None => DocumentCspOutcome::Allowed,
        }
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
        if let Some(popup_id) = self.lightweight_popup_id_for_document_handle(owner_document) {
            return Some(OwnerDispatchScope::LightweightPopup(popup_id));
        }
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
        let (mut report_only_violations, mut enforced_violations) = check.into_violations();
        if owner == OwnerDispatchScope::Top
            && let Some(position) = unsafe { &*self.runtime }.parser_script_start_position(script)
        {
            let line = i32::try_from(position.line).unwrap_or(i32::MAX);
            let column = i32::try_from(position.column).unwrap_or(i32::MAX);
            for violation in &mut report_only_violations {
                violation.line_number = line;
                violation.column_number = column;
            }
            for violation in &mut enforced_violations {
                violation.line_number = line;
                violation.column_number = column;
            }
        }
        let host_ptr: *mut JsContextHost = self;
        for violation in report_only_violations {
            self.dispatch_content_security_policy_violation_event_for_element_owner_best_effort(
                scope, host_ptr, owner, script, &violation,
            );
        }
        for violation in &enforced_violations {
            self.dispatch_content_security_policy_violation_event_for_element_owner_best_effort(
                scope, host_ptr, owner, script, violation,
            );
        }
        enforced_violations.is_empty()
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
        let (report_only_violations, enforced_violations) = check.into_violations();
        let host_ptr: *mut JsContextHost = self;
        for violation in report_only_violations {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        for violation in &enforced_violations {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, violation,
            );
        }
        enforced_violations.is_empty()
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

    pub(crate) fn check_document_connect_csp_for_owner_with_script_location<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        document_url: &url::Url,
        request_url: &url::Url,
    ) -> DocumentCspOutcome {
        let Some(check) = self.document_connect_csp_check_for_owner_with_redirect_status(
            owner,
            document_url,
            request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        ) else {
            return DocumentCspOutcome::Allowed;
        };
        if check.has_no_violations() {
            return DocumentCspOutcome::Allowed;
        }
        let location =
            crate::content_security_policy::ContentSecurityPolicySourceLocation::capture(scope);
        self.dispatch_document_connect_csp_check(scope, owner, check, Some(&location))
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
        self.dispatch_document_connect_csp_check(scope, owner, check, None)
    }

    fn dispatch_document_connect_csp_check<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: OwnerDispatchScope,
        check: DocumentContentSecurityPolicyCheck,
        source_location: Option<
            &crate::content_security_policy::ContentSecurityPolicySourceLocation,
        >,
    ) -> DocumentCspOutcome {
        let (mut report_only_violations, mut enforced_violations) = check.into_violations();
        if let Some(source_location) = source_location {
            for violation in report_only_violations
                .iter_mut()
                .chain(&mut enforced_violations)
            {
                source_location.apply_to(violation);
            }
        }
        let host_ptr: *mut JsContextHost = self;
        for violation in report_only_violations {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        for violation in &enforced_violations {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, violation,
            );
        }
        match enforced_violations.into_iter().next() {
            Some(violation) => DocumentCspOutcome::Blocked(violation),
            None => DocumentCspOutcome::Allowed,
        }
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
        .map(|check| check.into_violations().1.is_empty())
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

    fn child_effective_content_security_reporting_endpoints(
        &self,
        child_handle: DomHandle,
    ) -> ContentSecurityPolicyReportingEndpoints {
        self.child_browsing_contexts
            .get(&child_handle)
            .map(|entry| entry.content_security_reporting_endpoints())
            .unwrap_or_default()
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
        let (mut report_only_violations, mut enforced_violations) = check.into_violations();
        if report_only_violations.is_empty() && enforced_violations.is_empty() {
            return true;
        }
        if include_call_location
            && let Some((source_file, line_number, column_number)) =
                crate::content_security_policy::current_script_violation_location(scope)
        {
            for violation in [&mut report_only_violations, &mut enforced_violations]
                .into_iter()
                .flatten()
            {
                violation.source_file = source_file.clone();
                violation.line_number = line_number;
                violation.column_number = column_number;
            }
        }
        let host_ptr: *mut JsContextHost = self;
        for violation in report_only_violations {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, &violation,
            );
        }
        for violation in &enforced_violations {
            self.dispatch_content_security_policy_violation_event_for_owner_best_effort(
                scope, host_ptr, owner, violation,
            );
        }
        enforced_violations.is_empty()
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
        .into_iter()
        .next()
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
        let mut violations = unsafe { &*self.runtime }
            .trusted_type_policy_name_csp_violations_for_document(
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
        if !self.active_inspector_dispatch
            && let Some((source_file, line_number, column_number)) =
                crate::content_security_policy::current_script_violation_location(scope)
        {
            for violation in &mut violations {
                violation.source_file.clone_from(&source_file);
                violation.line_number = line_number;
                violation.column_number = column_number;
            }
        }
        let host_ptr: *mut JsContextHost = self;
        let allowed = !violations
            .iter()
            .any(|violation| violation.disposition == ContentSecurityPolicyDisposition::Enforce);
        // Policy creation reports expose CSP list ordering. Response policy
        // state is partitioned by disposition, with enforce policies modeled
        // before report-only policies, so preserve that order here.
        for violation in violations {
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
        let owner = policy_owner_dispatch_scope(scope);
        self.dispatch_trusted_types_sink_csp_violation_event_with_location_best_effort(
            scope, host_ptr, owner, sink, sample, true,
        );
    }

    pub(crate) fn dispatch_trusted_types_sink_csp_violation_event_for_global_best_effort<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        global: v8::Local<'s, v8::Object>,
        sink: &str,
        sample: &str,
    ) {
        let owner = policy_owner_dispatch_scope_for_global(scope, global);
        self.dispatch_trusted_types_sink_csp_violation_event_with_location_best_effort(
            scope, host_ptr, owner, sink, sample, true,
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
        let owner = policy_owner_dispatch_scope(scope);
        self.dispatch_trusted_types_sink_csp_violation_event_with_location_best_effort(
            scope, host_ptr, owner, sink, sample, false,
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
        owner: OwnerDispatchScope,
        sink: &str,
        sample: &str,
        capture_current_script_location: bool,
    ) {
        let source_location = (capture_current_script_location && !self.active_inspector_dispatch)
            .then(|| crate::content_security_policy::current_script_violation_location(scope))
            .flatten();
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
        let Some(document) = self.child_browsing_context_document_handle(handle) else {
            return;
        };
        let Some(owner) = self.current_child_document_task_owner(handle) else {
            return;
        };
        let host_ptr: *mut JsContextHost = self;
        // Preserve the source Document as the event target and enqueue the
        // event on its lifecycle, just as for a top-level Document.
        if let Err(error) = unsafe { &mut *self.runtime }
            .queue_content_security_policy_violation_event_for_target(
                scope,
                host_ptr,
                Some(document),
                violation,
                false,
                Some(owner),
            )
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
