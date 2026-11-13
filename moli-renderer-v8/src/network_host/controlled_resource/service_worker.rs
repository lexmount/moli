//! Service Worker selection beneath native XHR request/response decisions.
//! The retained native Request owns redirect, cookie, upload and policy state.

use std::sync::Arc;

use moli_fetch::{FetchCancelHandle, Request, ResponseHead};

use super::LoadedResponse;
use crate::{
    document_runtime::DocumentConnectPolicySnapshot,
    network::loads::ResourceLoadLease,
    service_worker_runtime::{
        ServiceWorkerClientId, ServiceWorkerDirectFetchCompletion, ServiceWorkerDirectFetchResult,
        ServiceWorkerDirectFetchStreamEvent, ServiceWorkerFetchDispatch, ServiceWorkerFetchRequest,
        ServiceWorkerRequestDestination, ServiceWorkerRuntimeService,
        service_worker_fetch_request_metadata,
    },
    types::{
        AsyncSubresourceNetworkContext, PendingSubresourceFetchInfo, SubresourcePolicyContext,
    },
};

pub(crate) struct XhrServiceWorkerContext {
    pub(super) runtime: ServiceWorkerRuntimeService,
    client_id: ServiceWorkerClientId,
    policy_context: SubresourcePolicyContext,
    pub(super) connect_policy: DocumentConnectPolicySnapshot,
    check_response:
        Arc<dyn Fn(&ResponseHead, moli_fetch::RedirectSource) -> Result<(), String> + Send + Sync>,
}

impl XhrServiceWorkerContext {
    pub(crate) fn new(
        runtime: ServiceWorkerRuntimeService,
        client_id: ServiceWorkerClientId,
        policy_context: SubresourcePolicyContext,
        connect_policy: DocumentConnectPolicySnapshot,
        check_response: impl Fn(&ResponseHead, moli_fetch::RedirectSource) -> Result<(), String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            runtime,
            client_id,
            policy_context,
            connect_policy,
            check_response: Arc::new(check_response),
        }
    }

    pub(super) fn check_response(
        &self,
        head: &ResponseHead,
        source: moli_fetch::RedirectSource,
    ) -> Result<(), String> {
        (self.check_response)(head, source)
    }

    pub(super) async fn load_hop(
        &self,
        load: &ResourceLoadLease,
        request: &mut Request,
        info: &PendingSubresourceFetchInfo,
        cancel: &FetchCancelHandle,
        preflight_headers: &mut Vec<(String, String)>,
    ) -> Result<Option<LoadedResponse>, String> {
        if load.request_client().bypass_service_worker()
            || self
                .runtime
                .matching_controller_for_client_fetch(self.client_id, &request.url)
                .is_none()
        {
            return Ok(None);
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let dispatch = ServiceWorkerFetchDispatch {
            request_body_owner: None,
            redirect_check: None, // The complete retained native Request already owns its check.
            internal_id: info.internal_id,
            request: ServiceWorkerFetchRequest {
                body_stream: None,
                client_id: Some(self.client_id),
                resulting_client_id: None,
                url: request.url.clone(),
                method: request.method.clone(),
                headers: request.request_headers.to_byte_strings(),
                body: request.body.clone(),
                destination: ServiceWorkerRequestDestination::Empty,
                request_mode: request.request_mode,
                credentials_mode: request.credentials_mode,
                redirect_mode: request.redirect_mode(),
                priority: request.priority_hints.fetch_priority,
                is_reload: false,
                metadata: service_worker_fetch_request_metadata(request),
            },
            cors_preflight_request_headers: preflight_headers.clone(),
            request_cookie_report: info.request_cookie_report.clone(),
            network_context: AsyncSubresourceNetworkContext {
                frame_id: info.frame_id.clone(),
                request_origin: request
                    .request_origin()
                    .cloned()
                    .expect("XHR retains its request origin"),
                document_url: info.document_url.clone(),
                resource_type: info.resource_type,
                policy_context: self.policy_context,
            },
            completion_tx:
                crate::page_task_queue::RendererResourceCompletionSender::direct_completion_only(),
            request_client: load.request_client(),
            resource_task_runner: load.task_runner(),
            cancel_handle: cancel.clone(),
            direct_completion_tx: Some(
                ServiceWorkerDirectFetchCompletion::streaming_with_caller_redirects(sender),
            ),
        };
        if !self
            .runtime
            .dispatch_controlled_fetch_hop(dispatch, request.clone())
        {
            return Ok(None);
        }
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("net::ERR_ABORTED".to_owned()),
            result = receiver => result.map_err(|_| "service worker XHR completion channel closed".to_owned())?,
        };
        match outcome {
            ServiceWorkerDirectFetchResult::Fallback => Ok(None),
            ServiceWorkerDirectFetchResult::NetworkFallback {
                request: next,
                preflight_headers: headers,
            } => {
                let upload_observer = request.upload_observer().cloned();
                *request = *next;
                if request.body.is_some()
                    && let Some(observer) = upload_observer
                {
                    *request = request.clone().with_upload_observer(observer);
                }
                *preflight_headers = headers;
                Ok(None)
            }
            ServiceWorkerDirectFetchResult::Response(response) => {
                let source = if response.from_network_fallback {
                    moli_fetch::RedirectSource::Network
                } else {
                    moli_fetch::RedirectSource::ServiceWorker
                };
                let filter = response.response_filter;
                let body_is_null = response.service_worker_body_is_null;
                let mut loaded = LoadedResponse::from_response((*response.response).into());
                loaded.source = source;
                loaded.response_filter = filter;
                loaded.body_is_null = body_is_null;
                Ok(Some(loaded))
            }
            ServiceWorkerDirectFetchResult::Stream(stream) => {
                let source = if stream
                    .response_filter
                    .as_ref()
                    .and_then(|filter| filter.service_worker_metadata())
                    .is_some()
                {
                    moli_fetch::RedirectSource::ServiceWorker
                } else {
                    moli_fetch::RedirectSource::Network
                };
                let head = stream.head;
                let filter = stream.response_filter;
                let chunks =
                    futures_util::stream::unfold(Some(stream.events), |state| async move {
                        let mut events = state?;
                        match events.recv().await {
                            Some(ServiceWorkerDirectFetchStreamEvent::Chunk(bytes)) => {
                                Some((Ok(bytes), Some(events)))
                            }
                            Some(ServiceWorkerDirectFetchStreamEvent::Finished(Ok(()))) => None,
                            Some(ServiceWorkerDirectFetchStreamEvent::Finished(Err(error))) => {
                                Some((Err(error), None))
                            }
                            None => Some((
                                Err("service worker XHR response stream closed".to_owned()),
                                None,
                            )),
                        }
                    });
                Ok(Some(LoadedResponse {
                    head,
                    body: crate::network::ResourceInterceptionBody::from_stream(
                        chunks,
                        cancel.clone(),
                        load.request_client().disk_pool(),
                    ),
                    response_filter: filter,
                    body_is_null: Some(false),
                    source,
                }))
            }
            ServiceWorkerDirectFetchResult::Failure(error) => Err(error),
            ServiceWorkerDirectFetchResult::FallbackStream(_) => {
                Err("XMLHttpRequest cannot upload an author stream".to_owned())
            }
        }
    }
}
