use super::super::window_security_tokens::WindowAccessOrigin;
use super::super::{ChildBrowsingContextSnapshot, JsContextHost};
use crate::document_script_scheduler::FrameDocumentClassicScriptSchedulerWork;
use crate::frame_owner_model::{
    DocumentCreationKind, FrameDocumentInteractiveLifecycleAction,
    FrameDocumentLocalWindowTransition, FrameLocalWindowOwnerTransition,
};
use crate::{document_runtime::DomHandle, frame_owner_model::FrameDocumentOwnerTransition};
use moli_web_mime::is_dom_parser_xml_mime;
use url::Url;

pub(crate) struct ChildDocumentInstallResult {
    pub(crate) initial_classic_ready_work: Option<FrameDocumentClassicScriptSchedulerWork>,
    pub(crate) parser_stop_action: Option<FrameDocumentInteractiveLifecycleAction>,
    pub(crate) owner_transition: FrameDocumentOwnerTransition,
}

#[derive(Clone, Debug)]
pub(in crate::native_bridge::context_host) struct ChildDocumentWindowCommitPreflight {
    expected_current_owner: Option<crate::frame_owner_model::FrameDocumentTaskOwner>,
    expected_current_document_handle: Option<DomHandle>,
    current_document_domain_was_set: bool,
    current_access_origin: Option<WindowAccessOrigin>,
}

#[derive(Clone, Debug)]
pub(in crate::native_bridge::context_host) struct ChildDocumentWindowCommit {
    expected_current_owner: Option<crate::frame_owner_model::FrameDocumentTaskOwner>,
    expected_current_document_handle: Option<DomHandle>,
    loader_id: Option<String>,
    origin: String,
    creation_kind: DocumentCreationKind,
    local_window_transition: FrameDocumentLocalWindowTransition,
}

impl ChildDocumentWindowCommitPreflight {
    pub(super) fn has_no_committed_document(&self) -> bool {
        self.expected_current_owner.is_none() && self.expected_current_document_handle.is_none()
    }
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn capture_child_document_window_commit_preflight(
        &self,
        handle: DomHandle,
    ) -> ChildDocumentWindowCommitPreflight {
        ChildDocumentWindowCommitPreflight {
            expected_current_owner: self
                .frame_owner_store
                .current_child_document_task_owner(handle),
            expected_current_document_handle: self
                .child_browsing_context_document_handles
                .get(&handle)
                .copied(),
            current_document_domain_was_set: self
                .child_browsing_context_document_domain_override(handle)
                .is_some(),
            current_access_origin: self.child_window_access_origin(handle),
        }
    }

    pub(in crate::native_bridge::context_host) fn plan_child_document_window_commit(
        &self,
        handle: DomHandle,
        snapshot: &ChildBrowsingContextSnapshot,
        preflight: ChildDocumentWindowCommitPreflight,
        creation_kind: DocumentCreationKind,
        loader_id: Option<String>,
    ) -> ChildDocumentWindowCommit {
        debug_assert!(
            !creation_kind.is_initial_empty(),
            "initial-empty frame initialization must not enter snapshot commit planning"
        );
        let origin = self
            .child_browsing_context_document_origin_for_url(handle, &snapshot.url)
            .unwrap_or_else(|| moli_url::origin_ascii_serialization(&snapshot.url));
        let current_document_domain_was_set = preflight.current_document_domain_was_set
            || self
                .child_browsing_context_document_domain_override(handle)
                .is_some();
        let security_origin_allows_reuse = !current_document_domain_was_set
            && preflight
                .current_access_origin
                .as_ref()
                .is_some_and(|current| {
                    self.prospective_child_window_access_origin(handle, &origin)
                        .as_ref()
                        .is_some_and(|prospective| current.can_access(prospective))
                });
        let local_window_transition = self
            .frame_owner_store
            .child_document_local_window_transition_for_commit(
                handle,
                preflight.expected_current_owner,
                security_origin_allows_reuse,
                &snapshot.policy_container,
            );
        ChildDocumentWindowCommit {
            expected_current_owner: preflight.expected_current_owner,
            expected_current_document_handle: preflight.expected_current_document_handle,
            loader_id,
            origin,
            creation_kind,
            local_window_transition,
        }
    }

    pub(in crate::native_bridge::context_host) fn child_document_window_commit_preflight_is_current(
        &self,
        handle: DomHandle,
        preflight: &ChildDocumentWindowCommitPreflight,
    ) -> bool {
        self.frame_owner_store
            .current_child_document_task_owner(handle)
            == preflight.expected_current_owner
            && self
                .child_browsing_context_document_handles
                .get(&handle)
                .copied()
                == preflight.expected_current_document_handle
    }

    pub(in crate::native_bridge::context_host) fn install_child_browsing_context_current_document_from_snapshot(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        snapshot: &ChildBrowsingContextSnapshot,
        window_commit: ChildDocumentWindowCommit,
        preserve_window_event_state: bool,
        navigation_loader: Option<crate::network::navigation::NavigationResourceLoader>,
    ) -> Option<ChildDocumentInstallResult> {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return None;
        }
        let is_xml_document = snapshot
            .content_type
            .as_deref()
            .is_some_and(is_dom_parser_xml_mime);
        self.install_live_child_document_from_snapshot(
            scope,
            handle,
            snapshot,
            window_commit,
            preserve_window_event_state,
            navigation_loader,
            is_xml_document,
        )
    }

    pub(in crate::native_bridge) fn replace_child_custom_elements_registry_for_document_commit(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        self.child_custom_elements.remove(&handle);
        if let Some(window) = self.existing_child_browsing_context_window_wrapper(scope, handle) {
            let _ = crate::custom_elements::rebind_materialized_child_custom_elements_registry(
                scope, window, handle,
            );
        }
    }

    fn install_live_child_document_from_snapshot(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        snapshot: &ChildBrowsingContextSnapshot,
        window_commit: ChildDocumentWindowCommit,
        preserve_window_event_state: bool,
        navigation_loader: Option<crate::network::navigation::NavigationResourceLoader>,
        is_xml_document: bool,
    ) -> Option<ChildDocumentInstallResult> {
        let is_plain_text_document = snapshot
            .content_type
            .as_deref()
            .is_some_and(|mime| mime.eq_ignore_ascii_case("text/plain"));
        let source = if is_xml_document {
            std::borrow::Cow::Borrowed(snapshot.markup.as_str())
        } else if is_plain_text_document {
            std::borrow::Cow::Owned(crate::dom_parser::plain_text_document_parser_input(
                &snapshot.markup,
            ))
        } else {
            crate::dom_parser::preserve_decoded_bom_only_browsing_context_body(
                &snapshot.markup,
                snapshot.content_type.as_deref(),
            )
        };
        let document_handle = if is_xml_document {
            self.create_empty_child_xml_document_from_snapshot(snapshot)
        } else {
            let document_handle = self.create_empty_live_child_html_document(
                snapshot.url.clone(),
                snapshot.content_type.as_deref(),
            );
            self.dom_host_mut()
                .set_document_fallback_base_url_for_handle(
                    document_handle,
                    snapshot.fallback_base_url.clone(),
                );
            document_handle
        };
        let document_url = self.document_url_for_handle(document_handle);
        let document_base_url = self.document_base_url_for_handle(document_handle);
        let parser_base_url = document_base_url.clone();
        let referrer_policy = self
            .child_browsing_context_referrer_policy_for_document_handle(document_handle)
            .map(str::to_owned);
        let owner_transition = self.commit_child_document_owner(
            scope,
            handle,
            document_handle,
            document_url,
            document_base_url,
            referrer_policy,
            snapshot.policy_container.clone(),
            window_commit,
            preserve_window_event_state,
            navigation_loader,
        )?;
        let current_owner = owner_transition
            .current_owner()
            .expect("child document commit must install a current owner");
        let owner_local_window_id = current_owner.local_window_id;
        let owner_document_id = current_owner.document_id;
        self.dom_host_mut()
            .mark_subtree_connected_preserving_owner_document(document_handle);
        self.install_empty_child_classic_script_runner_for_current_document(
            handle,
            owner_local_window_id,
            owner_document_id,
        );
        let parser_start = self.start_live_child_document_parser(
            scope,
            handle,
            document_handle,
            owner_local_window_id,
            owner_document_id,
            parser_base_url,
            source.as_ref(),
            is_xml_document,
        );
        Some(ChildDocumentInstallResult {
            initial_classic_ready_work: parser_start.initial_classic_ready_work,
            parser_stop_action: parser_start.parser_stop_action,
            owner_transition,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_child_document_owner(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        document_handle: DomHandle,
        document_url: Url,
        document_base_url: Url,
        referrer_policy: Option<String>,
        document_policy_container: crate::document_runtime::DocumentPolicyContainer,
        window_commit: ChildDocumentWindowCommit,
        preserve_window_event_state: bool,
        navigation_loader: Option<crate::network::navigation::NavigationResourceLoader>,
    ) -> Option<FrameDocumentOwnerTransition> {
        let ChildDocumentWindowCommit {
            expected_current_owner,
            expected_current_document_handle,
            loader_id,
            origin,
            creation_kind,
            local_window_transition,
        } = window_commit;
        let resource_context_origin = origin.clone();
        let current_document_handle = self
            .child_browsing_context_document_handles
            .get(&handle)
            .copied();
        if self
            .frame_owner_store
            .current_child_document_task_owner(handle)
            != expected_current_owner
            || current_document_handle != expected_current_document_handle
            || expected_current_owner.is_some() != expected_current_document_handle.is_some()
        {
            tracing::warn!(
                ?handle,
                ?expected_current_owner,
                ?expected_current_document_handle,
                ?current_document_handle,
                "aborting child document commit after its preflight owner became stale"
            );
            return None;
        }
        // Synthetic child Documents inherit the exact parent authority that
        // existed at the commit boundary. Capture it before retiring the old
        // child generation; never fall back to the ambient main Document.
        let inherited_resource_authority = navigation_loader.is_none().then(|| {
            self.parent_document_resource_loader_for_child_context(handle)
                .expect("synthetic child Document commit requires its parent authority")
                .clone()
        });
        let owner_transition = self
            .frame_owner_store
            .replace_child_document_with_local_window_transition(
                handle,
                document_handle,
                document_url,
                document_base_url,
                origin,
                referrer_policy,
                moli_fetch::RequestCredentialsMode::SameOrigin,
                document_policy_container.clone(),
                crate::types::SubresourcePolicyContext::from_document_policy(
                    &document_policy_container,
                ),
                creation_kind,
                local_window_transition,
                expected_current_owner,
            )?;
        debug_assert_eq!(owner_transition.retired_owner(), expected_current_owner);
        let ancestor_origins_refreshed =
            self.refresh_current_child_document_ancestor_origins(handle);
        debug_assert!(
            ancestor_origins_refreshed,
            "committed child Document must capture its ancestor origins"
        );

        match owner_transition.local_window_owner_transition() {
            FrameLocalWindowOwnerTransition::Replaced { .. } => {
                self.disconnect_shared_worker_clients_for_child_context(handle);
                if !preserve_window_event_state {
                    self.clear_child_window_document_event_state(scope, handle);
                }
                self.replace_child_custom_elements_registry_for_document_commit(scope, handle);
            }
            FrameLocalWindowOwnerTransition::Preserved { .. } => {
                self.replace_child_custom_elements_registry_for_document_commit(scope, handle);
            }
            FrameLocalWindowOwnerTransition::Installed { .. } => {}
            FrameLocalWindowOwnerTransition::Retired { .. } => {
                unreachable!("a child document commit must install a current LocalWindow")
            }
        }

        if let Some(retired_owner) = owner_transition.retired_owner() {
            self.retire_child_document_external_state(
                handle,
                retired_owner,
                expected_current_document_handle
                    .expect("retired owner must have a matching document handle"),
            );
        }

        let replaced_document_handle = self
            .child_browsing_context_document_handles
            .insert(handle, document_handle);
        debug_assert_eq!(
            replaced_document_handle, expected_current_document_handle,
            "adapter document handle must change in the same owner commit"
        );
        if let Some(loader_id) = loader_id {
            self.child_browsing_contexts
                .get_mut(&handle)
                .expect("committed child owner must retain its browsing context")
                .set_current_document_loader_id(loader_id);
        }
        let current_owner = owner_transition
            .current_owner()
            .expect("child document commit must install a current owner");
        let resource_authority = match navigation_loader {
            Some(loader) => {
                let seed = loader
                    .commit(self.document_url_for_handle(document_handle))
                    .expect("successful child navigation must commit its exact resource loader");
                crate::network::context::DocumentResourceAuthoritySource::Navigation(seed)
            }
            None => crate::network::context::DocumentResourceAuthoritySource::Inherited(
                inherited_resource_authority
                    .expect("synthetic child Document must capture its parent authority"),
            ),
        };
        self.register_committed_document_resource_loader(
            crate::network::context::DocumentFetchContext::new(
                crate::native_bridge::WindowDocumentOwner::Frame(current_owner),
                self.document_url_for_handle(document_handle),
                self.document_base_url_for_handle(document_handle),
                resource_context_origin,
            ),
            resource_authority,
        );
        Some(owner_transition)
    }

    fn create_empty_child_xml_document_from_snapshot(
        &mut self,
        snapshot: &ChildBrowsingContextSnapshot,
    ) -> DomHandle {
        let document_handle = self
            .dom_host_mut()
            .create_detached_xml_document_with_url(snapshot.url.clone());
        self.dom_host_mut()
            .set_document_fallback_base_url_for_handle(
                document_handle,
                snapshot.fallback_base_url.clone(),
            );
        if let Some(content_type) = snapshot.content_type.as_deref() {
            let _ = self.set_dom_document_content_type_for_handle(document_handle, content_type);
        }
        document_handle
    }
}
