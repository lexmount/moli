use super::super::{
    ChildBrowsingContextBootstrap, ChildBrowsingContextNavigationRequest,
    NavigationHistoryEntrySeed, PageScriptHost,
};
use crate::document_runtime::DomHandle;
use url::Url;

impl PageScriptHost {
    pub(crate) fn navigate_child_browsing_context_to_url(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
    ) -> bool {
        self.navigate_child_browsing_context_to_url_with_initiator(
            scope,
            handle,
            resolved_url,
            false,
        )
    }

    pub(crate) fn navigate_child_browsing_context_from_browser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
    ) -> bool {
        self.navigate_child_browsing_context_to_url_with_initiator(
            scope,
            handle,
            resolved_url,
            true,
        )
    }

    fn navigate_child_browsing_context_to_url_with_initiator(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
        browser_initiated: bool,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        if url.scheme() == "file" {
            let dispatch_scope = self.entered_owner_dispatch_scope(scope);
            let Some(source) = self.document_resource_loader_for_dispatch_scope(dispatch_scope)
            else {
                return false;
            };
            let client = source.request_client();
            let allows_local_files = !client.local_file_policy().blocks_file_urls()
                && (browser_initiated || client.has_document_file_access());
            if !allows_local_files {
                return false;
            }
            let context = source.fetch_context();
            let request = ChildBrowsingContextNavigationRequest {
                initiator: crate::runtime::RendererNavigationInitiator::new(
                    context.document_url().clone(),
                    context.request_origin(),
                    None,
                    allows_local_files,
                ),
                url,
                method: "GET".to_owned(),
                body: None,
                request_headers: Vec::new(),
            };
            return self.navigate_child_browsing_context_with_request(scope, handle, request);
        }
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        let replace_initial_empty_document = self.child_current_document_is_initial_empty(handle);
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            if replace_initial_empty_document {
                entry.replace_navigation_in_entry_seed(&url);
            } else {
                entry.apply_navigation_to_entry_seed(&url);
            }
        }
        self.sync_existing_child_browsing_context_runtime_surface_from_seed(scope, handle);
        let queued = self.queue_child_browsing_context_navigation_to_url(handle, &url);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }

    pub(crate) fn navigate_child_browsing_context_with_request(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        request: ChildBrowsingContextNavigationRequest,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let replace_initial_empty_document = self.child_current_document_is_initial_empty(handle);
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            if replace_initial_empty_document {
                entry.replace_navigation_in_entry_seed(&request.url);
            } else {
                entry.apply_navigation_to_entry_seed(&request.url);
            }
        }
        self.sync_existing_child_browsing_context_runtime_surface_from_seed(scope, handle);
        let queued = self.queue_child_browsing_context_navigation_request(handle, request);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }

    pub(crate) fn queue_child_browsing_context_navigation_from_existing_seed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
        replace_current: bool,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
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
        }
        let queued = self.queue_child_browsing_context_navigation_to_url(handle, &url);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }

    pub(crate) fn queue_child_browsing_context_navigation_without_seed_update(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        let queued = self.queue_child_browsing_context_navigation_to_url(handle, &url);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }

    pub(crate) fn queue_child_file_navigation_without_seed_update(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        url: Url,
        initiator: crate::runtime::RendererNavigationInitiator,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
        let request = ChildBrowsingContextNavigationRequest {
            url,
            method: "GET".to_owned(),
            body: None,
            request_headers: Vec::new(),
            initiator,
        };
        let queued = self.queue_child_browsing_context_navigation_request(handle, request);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }

    pub(crate) fn queue_deferred_child_browsing_context_navigation_from_entry_seed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
        entry_seed: NavigationHistoryEntrySeed,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.replace_navigation_entry_seed(entry_seed);
        }
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Url(url),
                false,
            )
            .is_none()
        {
            return false;
        }
        let queued = self.queue_child_browsing_context_navigation_commit(handle);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }

    pub(crate) fn cancel_joint_child_history_navigation(
        &mut self,
        handle: DomHandle,
        step: moli_session_history::SessionHistoryStepId,
    ) {
        if self
            .child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| {
                entry.navigation_entry_seed().session_history.target_step == Some(step)
            })
        {
            self.clear_child_browsing_context_pending_navigation(handle);
            if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
                entry.restore_navigation_entry_seed_from_committed();
            }
        }
    }

    pub(crate) fn queue_deferred_child_browsing_context_navigation_to_url(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.apply_deferred_navigation_to_entry_seed(&url);
        }
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Url(url),
                false,
            )
            .is_none()
        {
            return false;
        }
        let queued = self.queue_child_browsing_context_navigation_commit(handle);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }

    pub(crate) fn queue_deferred_child_browsing_context_navigation_request(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        request: ChildBrowsingContextNavigationRequest,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.apply_deferred_navigation_to_entry_seed(&request.url);
        }
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Request(request),
                false,
            )
            .is_none()
        {
            return false;
        }
        let queued = self.queue_child_browsing_context_navigation_commit(handle);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }

    pub(crate) fn queue_child_browsing_context_reload_from_existing_seed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        resolved_url: &str,
    ) -> bool {
        let download_policy = self.capture_child_navigation_download_policy(scope, handle);
        let Some(url) = Url::parse(resolved_url).ok() else {
            return false;
        };
        let queued = self.queue_child_browsing_context_navigation_to_url(handle, &url);
        self.remember_child_navigation_download_policy(handle, download_policy, queued)
    }
}
