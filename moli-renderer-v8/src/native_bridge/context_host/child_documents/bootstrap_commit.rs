use super::super::{ChildBrowsingContextBootstrap, ChildBrowsingContextSnapshot, JsContextHost};
use super::{ChildDocumentCommitResult, ChildDocumentNavigationInitiator};
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::{DocumentCreationKind, FrameDocumentNavigationLoadBinding},
};
use url::Url;

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn commit_child_document_bootstrap_or_start_load(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        bootstrap: ChildBrowsingContextBootstrap,
        navigation_load: FrameDocumentNavigationLoadBinding,
        initiator: ChildDocumentNavigationInitiator,
        initiator_url: Option<Url>,
    ) -> Option<ChildDocumentCommitResult> {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return None;
        }
        let creation_kind = child_document_creation_kind_for_bootstrap(&bootstrap);
        if let Some(request_url) = child_document_frame_csp_request_url(&bootstrap) {
            if let Some(violation) =
                self.frame_navigation_csp_report_only_violation(handle, request_url)
            {
                self.dispatch_frame_navigation_csp_violation_event_best_effort(
                    scope, handle, &violation,
                );
            }
            if let Some(violation) = self.frame_navigation_csp_violation(handle, request_url) {
                self.dispatch_frame_navigation_csp_violation_event_best_effort(
                    scope, handle, &violation,
                );
                self.cancel_child_document_navigation_after_csp_block(handle, navigation_load);
                return Some(ChildDocumentCommitResult::ready(None));
            }
        }
        self.cancel_child_meta_refresh_navigation(handle);
        let cached_snapshot =
            self.materialize_local_child_snapshot_for_bootstrap(handle, &bootstrap);
        let requires_async_load = cached_snapshot.is_none()
            && self.child_document_bootstrap_requires_async_load(&bootstrap);
        let owner_credentialless = self
            .child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.owner_credentialless());
        let document_credentialless = self
            .child_browsing_context_document_credentialless_for_owner(handle, owner_credentialless);
        let credentialless_storage_nonce =
            self.child_document_credentialless_storage_nonce(document_credentialless);

        self.clear_pending_child_document_loads_for_handle(handle);

        if requires_async_load {
            if self
                .start_child_document_load(
                    handle,
                    bootstrap.clone(),
                    navigation_load,
                    initiator,
                    initiator_url,
                )
                .is_some()
            {
                return Some(ChildDocumentCommitResult::pending());
            }
            let url = Self::child_browsing_context_bootstrap_url(&bootstrap)?;
            tracing::debug!(
                handle = ?handle,
                url = %url,
                "child document load could not start"
            );
            let snapshot = ChildBrowsingContextSnapshot::html(
                url,
                "<!DOCTYPE html><html><head></head><body></body></html>".to_owned(),
            );
            let window_commit_preflight =
                self.capture_child_document_window_commit_preflight(handle);
            self.dispatch_child_browsing_context_unload_lifecycle_if_needed(scope, handle);
            if !self
                .child_document_window_commit_preflight_is_current(handle, &window_commit_preflight)
            {
                let _ = self
                    .finish_child_frame_navigation_without_load_dispatch(handle, navigation_load);
                return Some(ChildDocumentCommitResult::ready(None));
            }
            let sandbox = self.child_browsing_context_sandbox_policy_from_owner(handle);
            {
                let entry = self.child_browsing_contexts.get_mut(&handle)?;
                entry.commit_child_document_after_failed_async_start(
                    bootstrap,
                    &snapshot,
                    sandbox,
                    document_credentialless,
                    credentialless_storage_nonce,
                );
            }
            let snapshot =
                self.cache_child_snapshot_with_current_document_policy(handle, snapshot)?;
            let loader_id = self.allocate_child_document_loader_id();
            let window_commit = self.plan_child_document_window_commit(
                handle,
                &snapshot,
                window_commit_preflight,
                creation_kind,
                Some(loader_id),
            );
            let install = self.install_child_browsing_context_current_document_from_snapshot(
                scope,
                handle,
                &snapshot,
                window_commit,
                false,
                None,
            )?;
            self.promote_pending_service_worker_child_client(handle);
            self.register_or_update_service_worker_child_client(handle);
            self.complete_pending_service_worker_child_client_navigation(handle);
            return Some(ChildDocumentCommitResult::from_install(install));
        }

        let sandbox = self.child_browsing_context_sandbox_policy_from_owner(handle);
        let window_commit_preflight = self.capture_child_document_window_commit_preflight(handle);
        self.dispatch_child_browsing_context_unload_lifecycle_if_needed(scope, handle);
        if !self.child_document_window_commit_preflight_is_current(handle, &window_commit_preflight)
        {
            let _ =
                self.finish_child_frame_navigation_without_load_dispatch(handle, navigation_load);
            return Some(ChildDocumentCommitResult::ready(None));
        }
        {
            let entry = self.child_browsing_contexts.get_mut(&handle)?;
            entry.commit_new_child_document(
                bootstrap,
                cached_snapshot.as_ref(),
                sandbox,
                document_credentialless,
                credentialless_storage_nonce,
            );
        }

        let result = if let Some(snapshot) = cached_snapshot {
            let snapshot =
                self.cache_child_snapshot_with_current_document_policy(handle, snapshot)?;
            let loader_id = self.allocate_child_document_loader_id();
            let window_commit = self.plan_child_document_window_commit(
                handle,
                &snapshot,
                window_commit_preflight,
                creation_kind,
                Some(loader_id),
            );
            let install = self.install_child_browsing_context_current_document_from_snapshot(
                scope,
                handle,
                &snapshot,
                window_commit,
                false,
                None,
            )?;
            ChildDocumentCommitResult::from_install(install)
        } else {
            self.disconnect_shared_worker_clients_for_child_context(handle);
            self.clear_child_window_document_event_state(scope, handle);
            self.replace_child_custom_elements_registry_for_document_commit(scope, handle);
            self.clear_child_browsing_context_current_document(handle);
            ChildDocumentCommitResult::ready(None)
        };
        self.promote_pending_service_worker_child_client(handle);
        self.register_or_update_service_worker_child_client(handle);
        self.complete_pending_service_worker_child_client_navigation(handle);
        Some(result)
    }

    pub(in crate::native_bridge::context_host) fn cache_child_snapshot_with_current_document_policy(
        &mut self,
        handle: DomHandle,
        mut snapshot: ChildBrowsingContextSnapshot,
    ) -> Option<ChildBrowsingContextSnapshot> {
        let entry = self.child_browsing_contexts.get_mut(&handle)?;
        snapshot.policy_container = entry.document_policy_container_snapshot();
        entry.cache_snapshot(snapshot.clone());
        Some(snapshot)
    }

    fn cancel_child_document_navigation_after_csp_block(
        &mut self,
        handle: DomHandle,
        navigation_load: FrameDocumentNavigationLoadBinding,
    ) {
        let _ = self.finish_child_frame_navigation_without_load_dispatch(handle, navigation_load);
        self.clear_pending_child_document_loads_for_handle(handle);
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "Cannot navigate to URL.".to_owned(),
        );
        self.clear_child_browsing_context_pending_navigation(handle);
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.clear_pending_document_load();
            entry.clear_pending_top_level_history_length_increment();
            entry.restore_navigation_entry_seed_from_committed();
        }
    }
}

fn child_document_creation_kind_for_bootstrap(
    bootstrap: &ChildBrowsingContextBootstrap,
) -> DocumentCreationKind {
    match bootstrap {
        ChildBrowsingContextBootstrap::Srcdoc { .. } => DocumentCreationKind::Srcdoc,
        ChildBrowsingContextBootstrap::AboutBlank
        | ChildBrowsingContextBootstrap::Url(_)
        | ChildBrowsingContextBootstrap::Request(_) => DocumentCreationKind::Navigation,
    }
}

fn child_document_frame_csp_request_url(bootstrap: &ChildBrowsingContextBootstrap) -> Option<&Url> {
    let url = match bootstrap {
        ChildBrowsingContextBootstrap::Url(url) => url,
        ChildBrowsingContextBootstrap::Request(request) => &request.url,
        ChildBrowsingContextBootstrap::AboutBlank
        | ChildBrowsingContextBootstrap::Srcdoc { .. } => {
            return None;
        }
    };
    if moli_url::is_about_blank(url) || url.scheme() == "javascript" {
        return None;
    }
    Some(url)
}
