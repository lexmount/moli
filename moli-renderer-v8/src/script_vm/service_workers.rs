use super::ScriptVm;
use crate::service_worker_runtime::ServiceWorkerClientNavigateError;

impl ScriptVm {
    pub(crate) fn service_worker_subresource_fetch_context(
        &self,
        request_url: &url::Url,
    ) -> Option<(
        crate::runtime::RendererBrowserContextRuntime,
        crate::service_worker_runtime::ServiceWorkerClientId,
    )> {
        let host = self._context_host.borrow();
        let client_id = host.service_worker_client_id_for_window_fetch(None);
        let document_url = self.document_runtime.document_url();
        host.service_worker_controller_for_fetch(client_id, document_url, request_url)?;
        Some((host.browser_context_runtime(), client_id))
    }

    pub(crate) fn complete_pending_service_worker_client_navigate_after_follow(
        &mut self,
        continuation: crate::types::ServiceWorkerClientNavigateContinuation,
    ) {
        let (browser_context_runtime, client_id) = {
            let context_host = self._context_host.borrow();
            (
                context_host.browser_context_runtime(),
                context_host.service_worker_client_id(),
            )
        };
        let result = browser_context_runtime
            .service_worker_runtime()
            .client_navigate_result_for_current_window_client(
                continuation.source_version_id,
                client_id,
            );
        browser_context_runtime
            .service_worker_runtime()
            .enqueue_client_navigate_completed(
                crate::types::ServiceWorkerClientNavigateCompletion {
                    request_id: continuation.request_id,
                    source_version_id: continuation.source_version_id,
                    source_run: continuation.source_run,
                    result,
                },
            );
    }

    pub(crate) fn unregister_reserved_service_worker_client_after_navigation_abort(
        &mut self,
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
    ) {
        let browser_context_runtime = self._context_host.borrow().browser_context_runtime();
        browser_context_runtime.unregister_service_worker_client(client_id);
    }

    pub(crate) fn reject_pending_service_worker_client_navigate_after_follow(
        &mut self,
        continuation: crate::types::ServiceWorkerClientNavigateContinuation,
        message: String,
    ) {
        let browser_context_runtime = self._context_host.borrow().browser_context_runtime();
        browser_context_runtime
            .service_worker_runtime()
            .enqueue_client_navigate_completed(
                crate::types::ServiceWorkerClientNavigateCompletion {
                    request_id: continuation.request_id,
                    source_version_id: continuation.source_version_id,
                    source_run: continuation.source_run,
                    result: Err(ServiceWorkerClientNavigateError::type_error(message)),
                },
            );
    }
}
