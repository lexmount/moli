use super::*;
use crate::service_worker_runtime::{
    ServiceWorkerDirectFetchResult, ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
    service_worker_fetch_request_metadata,
};

impl ScriptVm {
    /// Initial Fetch.continueRequest for a capture-only resource. Auth retries
    /// intentionally use the ordinary buffered transport entry, not this method.
    pub(super) fn spawn_running_capture_resource_fetch(
        &mut self,
        request_client: ResourceRequestClient,
        request: moli_fetch::Request,
        state: RunningSubresourceFetchState,
        cancel_handle: moli_fetch::FetchCancelHandle,
    ) -> Result<()> {
        let PendingSubresourceContinuation::CaptureResource(capture) = &state.pending.continuation
        else {
            bail!("capture continuation requires a capture resource destination");
        };
        let service_worker = {
            let host = self._context_host.borrow();
            if !capture.endpoint.is_active()
                || host.current_window_document_task_target_for_dispatch_scope(
                    capture.target.dispatch_scope(),
                ) != Some(capture.target)
            {
                bail!("capture resource Document owner changed while request was paused");
            }
            let client_id = host
                .service_worker_client_id_for_subresource_owner(capture.target.dispatch_scope());
            if matches!(request.url.scheme(), "http" | "https")
                && host
                    .service_worker_controller_for_fetch(
                        client_id,
                        &state.pending.info.document_url,
                        &request.url,
                    )
                    .is_some()
            {
                let destination = match capture.kind {
                    crate::capture_resources::CaptureResourceKind::Font => {
                        ServiceWorkerRequestDestination::Font
                    }
                    crate::capture_resources::CaptureResourceKind::Image => {
                        ServiceWorkerRequestDestination::Image
                    }
                };
                let (direct_completion_tx, receiver) = tokio::sync::oneshot::channel();
                Some((
                    ServiceWorkerFetchDispatch {
                        internal_id: state.pending.info.internal_id,
                        request: host.service_worker_fetch_request(
                            client_id,
                            request.url.clone(),
                            request.method.clone(),
                            request.request_headers.clone(),
                            request.body.clone(),
                            destination,
                            request.request_mode,
                            request.credentials_mode,
                            request.redirect_mode,
                            request.priority_hints.fetch_priority,
                            service_worker_fetch_request_metadata(&request),
                        ),
                        cors_preflight_request_headers: request.request_headers.to_byte_strings(),
                        request_cookie_report: state.pending.info.request_cookie_report.clone(),
                        network_context: crate::types::AsyncSubresourceNetworkContext {
                            frame_id: state.pending.info.frame_id.clone(),
                            request_origin: state.pending.request_origin.clone(),
                            document_url: state.pending.info.document_url.clone(),
                            resource_type: state.pending.info.resource_type,
                            policy_context: state.pending.policy_context,
                        },
                        completion_tx: host.resource_completion_sender(),
                        request_client: request_client.clone(),
                        resource_task_runner: state.pending.load.task_runner(),
                        cancel_handle: cancel_handle.clone(),
                        direct_completion_tx: Some(direct_completion_tx),
                    },
                    receiver,
                ))
            } else {
                None
            }
        };
        let Some((dispatch, receiver)) = service_worker else {
            self.spawn_running_subresource_fetch(
                request_client,
                request,
                state,
                Some(cancel_handle),
            );
            return Ok(());
        };

        let task_runner = state.pending.load.task_runner();
        let internal_id = state.pending.info.internal_id;
        let request_url = state.request_url.clone();
        let request_method = state.request_method.clone();
        let request_headers = state.request_headers.clone();
        let request_body = state.request_body.clone();
        let completion_tx = self._context_host.borrow().resource_completion_sender();
        {
            let mut host = self._context_host.borrow_mut();
            host.begin_active_subresource_request();
            host.record_running_subresource_fetch(state);
        }
        if !self
            ._context_host
            .borrow()
            .dispatch_service_worker_fetch_with_prepared_request(dispatch, request.clone())
        {
            let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: Err("service worker capture request dispatch failed".to_owned()).into(),
            });
            return Ok(());
        }

        // Only owned request data crosses this await. The pending Running state
        // retains the original Document lease and exact capture capability.
        // All terminals return through normal Page completion and Fetch response/
        // auth processing; this task never delivers directly to the job mailbox.
        task_runner.spawn(async move {
            let (result, skip_fetch_security_validation, response_filter, response_status_text) =
                match receiver.await {
                    Ok(ServiceWorkerDirectFetchResult::Fallback) => {
                        // Keep the prepared request, including redirect overrides,
                        // partition and page policy. Running/response interception
                        // buffers the body; it must not discard an Image sidecar.
                        let result = crate::network_host::fetch_browser_subresource_with_preflight_and_network_metadata(
                            request_client,
                            request,
                            Some(cancel_handle),
                        )
                        .await
                        .map(|observed| {
                            let (response, observation) = observed.into_parts();
                            crate::protocol_types::NavigationResponse::from(response)
                                .with_network_request_headers(
                                    observation.map(|observation| observation.into_headers()),
                                )
                        });
                        (result, false, None, None)
                    }
                    Ok(ServiceWorkerDirectFetchResult::Response(response)) => {
                        let status_text = response.response.head().status_text;
                        (
                            Ok(*response.response),
                            response.skip_fetch_security_validation,
                            response.response_filter,
                            status_text,
                        )
                    }
                    Ok(ServiceWorkerDirectFetchResult::Failure(error)) => {
                        (Err(error), false, None, None)
                    }
                    Err(_) => (
                        Err("service worker capture completion channel closed".to_owned()),
                        false,
                        None,
                        None,
                    ),
                };
            let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text,
                skip_fetch_security_validation,
                response_filter,
                network_error_text: None,
                result: result.into(),
            });
        });
        Ok(())
    }
}
