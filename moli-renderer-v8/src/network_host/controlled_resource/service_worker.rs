use super::*;
use crate::service_worker_runtime::{
    ServiceWorkerClientId, ServiceWorkerDirectFetchResult, ServiceWorkerFetchDispatch,
    ServiceWorkerFetchRequest, ServiceWorkerFetchRequestMetadata, ServiceWorkerRequestDestination,
    ServiceWorkerRuntimeService,
};

#[derive(Clone)]
pub(crate) struct WorkerServiceWorkerFetch {
    pub(crate) runtime: ServiceWorkerRuntimeService,
    pub(crate) client_id: ServiceWorkerClientId,
    pub(crate) metadata: ServiceWorkerFetchRequestMetadata,
    body_stream: Option<crate::context_bootstrap::ReadableStreamClonePayload>,
    request_body_owner: Option<crate::context_bootstrap::RequestBodyOwner>,
}
impl ControlledResourceLoad {
    pub(crate) fn with_service_worker(
        mut self,
        runtime: Option<ServiceWorkerRuntimeService>,
        client_id: Option<ServiceWorkerClientId>,
        metadata: ServiceWorkerFetchRequestMetadata,
    ) -> Self {
        if let (Some(runtime), Some(client_id)) = (runtime, client_id) {
            self.service_worker = Some(WorkerServiceWorkerFetch {
                runtime,
                client_id,
                metadata,
                body_stream: None,
                request_body_owner: None,
            });
        }
        self
    }
    pub(crate) fn with_service_worker_request_body(mut self, body_stream: Option<crate::context_bootstrap::ReadableStreamClonePayload>, request_body_owner: Option<crate::context_bootstrap::RequestBodyOwner>) -> Self {
        if let Some(source) = &mut self.service_worker {
            source.body_stream = body_stream;
            source.request_body_owner = request_body_owner;
        }
        self
    }
    pub(super) async fn fetch_service_worker_hop(
        &mut self,
        source: WorkerServiceWorkerFetch,
    ) -> Result<Option<LoadedResponse>, String> {
        if self.load.request_client().bypass_service_worker()
            || source
                .runtime
                .matching_controller_for_client_fetch(source.client_id, &self.request.url)
                .is_none()
        {
            return Ok(None);
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ResourceResponsePolicy::Fetch(policy_context) = self.response_policy else {
            return Ok(None);
        };
        let dispatch = ServiceWorkerFetchDispatch {
            redirect_check: None,
            request_body_owner: source.request_body_owner,
            internal_id: self.info.internal_id,
            request: ServiceWorkerFetchRequest {
                body_stream: self.service_worker.as_mut().and_then(|source| source.body_stream.take()),
                client_id: source.client_id,
                resulting_client_id: None,
                url: self.request.url.clone(),
                method: self.request.method.clone(),
                headers: self.request.request_headers.to_byte_strings(),
                body: self.request.body.clone(),
                destination: ServiceWorkerRequestDestination::for_subresource_resource_type(
                    self.info.resource_type,
                )
                .unwrap_or(ServiceWorkerRequestDestination::Empty),
                request_mode: self.request.request_mode,
                credentials_mode: self.request.credentials_mode,
                redirect_mode: self.request.redirect_mode(),
                priority: self.request.priority_hints.fetch_priority,
                is_reload: false,
                metadata: source.metadata,
            },
            cors_preflight_request_headers: self.preflight_headers.clone(),
            request_cookie_report: None,
            network_context: crate::types::AsyncSubresourceNetworkContext {
                frame_id: None,
                document_url: self.info.document_url.clone(),
                request_origin: self
                    .request
                    .browser_origin()
                    .map_err(|e| e.to_string())?
                    .clone(),
                resource_type: self.info.resource_type,
                policy_context,
            },
            completion_tx:
                crate::page_task_queue::RendererResourceCompletionSender::direct_completion_only(),
            request_client: self.load.request_client(),
            resource_task_runner: self.load.task_runner(),
            cancel_handle: self.cancel.clone(),
            direct_completion_tx: Some(tx),
        };
        if !source
            .runtime
            .dispatch_controlled_fetch_hop(dispatch, self.request.clone())
        {
            return Err("service worker fetch dispatch failed".into());
        }
        let result = tokio::select! {
            biased;
            () = self.cancel.cancelled() => { source.runtime.abort_controlled_fetch(self.info.internal_id); return Err("net::ERR_ABORTED".into()); }
            result = rx => result.map_err(|_|"service worker response channel closed".to_owned())?,
        };
        match result {
            ServiceWorkerDirectFetchResult::Fallback => Ok(None),
            ServiceWorkerDirectFetchResult::FallbackStream(body) => {
                self.request = self.request.clone().with_upload_body(body);
                Ok(None)
            }
            ServiceWorkerDirectFetchResult::Stream(stream) => {
                self.response_filter = stream.response_filter;
                Ok(Some(LoadedResponse { head: stream.head, body: crate::network::ResourceInterceptionBody::from_service_worker_stream(stream.events, self.load.request_client().disk_pool()) }))
            }
            ServiceWorkerDirectFetchResult::Failure(error) => Err(error),
            ServiceWorkerDirectFetchResult::Response(response) => {
                self.response_filter = response.response_filter;
                Ok(Some(LoadedResponse::from_response((*response.response).into())))
            }
        }
    }
}
