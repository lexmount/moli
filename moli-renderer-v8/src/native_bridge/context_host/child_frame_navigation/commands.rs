use super::super::{
    ChildBrowsingContextBootstrap, ChildBrowsingContextNavigationRequest, JsContextHost,
    NavigationHistoryEntrySeed,
};
use crate::document_runtime::DomHandle;
use moli_page_types::{NavigationHistoryMutation, cross_document_navigation_seed};
use url::Url;

impl JsContextHost {
    pub(crate) fn queue_deferred_child_form_navigation_request(
        &mut self,
        handle: DomHandle,
        request: ChildBrowsingContextNavigationRequest,
        entry_seed: Option<NavigationHistoryEntrySeed>,
        mutation: NavigationHistoryMutation,
    ) -> bool {
        let Some(entry) = self.child_browsing_contexts.get(&handle) else {
            return false;
        };
        if request.url.scheme() == "javascript" {
            return self.queue_child_browsing_context_navigation_without_seed_update(
                handle,
                request.url.as_str(),
                None,
            );
        }
        let entry_seed = entry_seed.unwrap_or_else(|| {
            // A named target need not have a Window wrapper yet. Use its
            // committed history, never a previously planned form navigation.
            let committed = entry.committed_navigation_entry_seed();
            let index = committed
                .entries
                .iter()
                .find(|entry| entry.history_index == committed.current_index)
                .map_or(0, |entry| entry.index);
            cross_document_navigation_seed(
                committed.entries,
                committed.current_index,
                index,
                &request.url,
                mutation,
            )
        });
        let increments_joint_history = mutation == NavigationHistoryMutation::Push;
        if request.method == "GET" {
            return self.queue_deferred_child_browsing_context_navigation_from_entry_seed(
                handle,
                request.url.as_str(),
                entry_seed,
                increments_joint_history,
                None,
            );
        }
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.replace_navigation_entry_seed_and_clear_pending_history_increment(entry_seed);
            if increments_joint_history {
                entry.mark_pending_top_level_history_length_increment();
            }
        }
        let url = request.url.clone();
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Request(request),
                None,
                false,
            )
            .is_none()
        {
            return false;
        }
        self.register_reserved_service_worker_child_client_for_navigation(handle, &url);
        self.queue_child_browsing_context_navigation_commit(handle)
    }

    pub(crate) fn navigate_child_browsing_context_to_url(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        let replace_initial_empty_document =
            self.child_browsing_context_is_on_initial_about_blank_entry(handle);
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            if replace_initial_empty_document {
                entry.replace_navigation_in_entry_seed(&url);
            } else {
                entry.apply_navigation_to_entry_seed(&url);
                entry.mark_pending_top_level_history_length_increment();
            }
        }
        self.sync_existing_child_browsing_context_runtime_surface_from_seed(scope, handle);
        self.queue_child_browsing_context_navigation_to_url(handle, &url, None)
    }

    pub(crate) fn navigate_child_browsing_context_with_request(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        request: ChildBrowsingContextNavigationRequest,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let replace_initial_empty_document =
            self.child_browsing_context_is_on_initial_about_blank_entry(handle);
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            if replace_initial_empty_document {
                entry.replace_navigation_in_entry_seed(&request.url);
            } else {
                entry.apply_navigation_to_entry_seed(&request.url);
                entry.mark_pending_top_level_history_length_increment();
            }
        }
        self.sync_existing_child_browsing_context_runtime_surface_from_seed(scope, handle);
        self.queue_child_browsing_context_navigation_request(handle, request)
    }

    pub(crate) fn queue_child_browsing_context_navigation_from_existing_seed(
        &mut self,
        handle: DomHandle,
        resolved_url: &str,
        replace_current: bool,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.apply_queued_navigation_to_entry_seed(&url, replace_current);
            if !replace_current {
                entry.mark_pending_top_level_history_length_increment();
            }
        }
        self.queue_child_browsing_context_navigation_to_url(handle, &url, None)
    }

    pub(crate) fn queue_child_browsing_context_navigation_without_seed_update(
        &mut self,
        handle: DomHandle,
        resolved_url: &str,
        initiator_url: Option<Url>,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        self.queue_child_browsing_context_navigation_to_url(handle, &url, initiator_url)
    }

    pub(crate) fn queue_deferred_child_browsing_context_navigation_from_entry_seed(
        &mut self,
        handle: DomHandle,
        resolved_url: &str,
        entry_seed: NavigationHistoryEntrySeed,
        increments_joint_history: bool,
        initiator_url: Option<Url>,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.replace_navigation_entry_seed_and_clear_pending_history_increment(entry_seed);
            if increments_joint_history {
                entry.mark_pending_top_level_history_length_increment();
            }
        }
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Url(url.clone()),
                initiator_url,
                false,
            )
            .is_none()
        {
            return false;
        }
        self.register_reserved_service_worker_child_client_for_navigation(handle, &url);
        self.queue_child_browsing_context_navigation_commit(handle)
    }

    pub(crate) fn child_browsing_context_has_pending_cross_document_traversal(
        &self,
        handle: DomHandle,
    ) -> bool {
        let Some(entry) = self.child_browsing_contexts.get(&handle) else {
            return false;
        };
        if !entry.has_pending_navigation_or_document_load() {
            return false;
        }
        let pending = entry.navigation_entry_seed();
        // The pending activation describes the destination, not the active
        // Document's last navigation. A traversal must also change position.
        pending.current_index != entry.committed_navigation_entry_seed().current_index
            && pending
                .activation
                .as_ref()
                .and_then(|activation| activation.navigation_type.as_deref())
                == Some("traverse")
    }

    pub(crate) fn cancel_pending_child_browsing_context_navigation(&mut self, handle: DomHandle) {
        let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
            return;
        };
        if !entry.has_pending_navigation_or_document_load() {
            return;
        }
        entry.clear_pending_navigation();
        entry.clear_pending_top_level_history_length_increment();
        entry.restore_navigation_entry_seed_from_committed();
        self.retire_current_child_navigation_commit_task(handle);
        self.clear_pending_form_submission_child_target(handle);
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        if let Some(navigation_load) = self.current_child_navigation_load(handle) {
            let _ =
                self.finish_child_frame_navigation_without_load_dispatch(handle, navigation_load);
        }
        // Removing the request's owner also makes an already-arriving network
        // completion stale; it must never install a Document after cancellation.
        self.clear_pending_child_document_loads_for_handle(handle);
    }
}
