//! Test-only identities for producing real ServiceWorker internal tasks.
//!
//! This module exposes pending identities already created by Web API entry
//! points. It does not settle requests, dispatch events, perform checkpoints,
//! claim Page tasks, or duplicate the Page arbiter. Complete behavior tests
//! must consume these identities through the production selected-task
//! dispatcher.

use super::ScriptVm;

impl ScriptVm {
    pub(crate) fn service_worker_ready_request_for_test(
        &self,
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    ) -> Option<(u64, crate::native_bridge::WindowDocumentOwner)> {
        self._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .into_iter()
            .find(|(_, owner)| owner.dispatch_scope() == dispatch_scope)
            .map(|(request_id, owner)| (request_id, owner.window_document_owner()))
    }

    pub(crate) fn service_worker_lifecycle_watcher_for_test(
        &self,
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
        scope_url: &url::Url,
    ) -> Option<(crate::native_bridge::WindowDocumentOwner, String)> {
        self._context_host
            .borrow()
            .service_worker_registration_watchers_for_test()
            .into_iter()
            .find(|(owner, watcher_scope, _)| {
                owner.dispatch_scope() == dispatch_scope && watcher_scope == scope_url
            })
            .map(|(owner, _, storage_key)| (owner.window_document_owner(), storage_key))
    }

    pub(crate) fn service_worker_internal_window_client_target_for_test(
        &self,
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    ) -> Option<crate::types::ServiceWorkerWindowClientTarget> {
        self._context_host
            .borrow()
            .service_worker_window_client_target_for_test(dispatch_scope)
    }
}
