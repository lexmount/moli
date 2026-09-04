use super::super::{
    ChildBrowsingContextBootstrap, ChildBrowsingContextNavigationRequest, JsContextHost,
    NavigationHistoryEntrySeed,
};
use crate::document_runtime::DomHandle;
use url::Url;

impl JsContextHost {
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
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.replace_navigation_entry_seed_and_clear_pending_history_increment(entry_seed);
            if increments_joint_history {
                entry.mark_pending_top_level_history_length_increment();
            }
        }
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Url(url),
                None,
                false,
            )
            .is_none()
        {
            return false;
        }
        self.queue_child_browsing_context_navigation_commit(handle)
    }

    pub(crate) fn queue_deferred_child_browsing_context_navigation_to_url(
        &mut self,
        handle: DomHandle,
        resolved_url: &str,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.apply_deferred_navigation_to_entry_seed(&url);
            entry.mark_pending_top_level_history_length_increment();
        }
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Url(url),
                None,
                false,
            )
            .is_none()
        {
            return false;
        }
        self.queue_child_browsing_context_navigation_commit(handle)
    }

    pub(crate) fn queue_deferred_child_browsing_context_navigation_request(
        &mut self,
        handle: DomHandle,
        request: ChildBrowsingContextNavigationRequest,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.apply_deferred_navigation_to_entry_seed(&request.url);
            entry.mark_pending_top_level_history_length_increment();
        }
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
        self.queue_child_browsing_context_navigation_commit(handle)
    }

    pub(crate) fn mark_child_browsing_context_top_level_history_increment(
        &mut self,
        handle: DomHandle,
    ) {
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.mark_pending_top_level_history_length_increment();
        }
    }

    pub(crate) fn queue_child_browsing_context_reload_from_existing_seed(
        &mut self,
        handle: DomHandle,
        resolved_url: &str,
        initiator_url: Option<Url>,
    ) -> bool {
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        self.queue_child_browsing_context_navigation_to_url(handle, &url, initiator_url)
    }
}
