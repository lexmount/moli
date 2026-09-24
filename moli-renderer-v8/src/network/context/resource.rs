use moli_fetch::{FetchCancelHandle, Request};

use crate::{
    network::{ResourceResponseFailure, ResourceResponseStream},
    protocol_types::NavigationResponse,
    runtime::RendererBrowserContextRuntime,
    service_worker_runtime::{ServiceWorkerClientId, ServiceWorkerRequestDestination},
    types::{
        AsyncSubresourceFetchResponseFilter, SubresourceRequestInitiatorType,
        SubresourceResourceType,
    },
};

use super::DocumentResourceLoader;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceResponseProvenance {
    Network,
    ServiceWorker {
        filter: Option<AsyncSubresourceFetchResponseFilter>,
    },
}

// Bind the client's resource operation without making every transport/cache
// holder depend on the full BrowserContext service graph.
pub(super) type ServiceWorkerResourceFetcher = dyn Fn(
        Request,
        SubresourceResourceType,
        &crate::network::loads::ResourceLoadLease,
        FetchCancelHandle,
    ) -> Option<
        futures_util::future::BoxFuture<
            'static,
            anyhow::Result<Option<crate::service_worker_runtime::ServiceWorkerResourceResponse>>,
        >,
    > + Send
    + Sync;

impl DocumentResourceLoader {
    #[cfg(test)]
    pub(crate) async fn fetch_script_for_test(
        &self,
        request: Request,
    ) -> anyhow::Result<NavigationResponse> {
        self.fetch_resource(
            request,
            SubresourceResourceType::Script,
            SubresourceRequestInitiatorType::Parser,
        )
        .await
        .map(|(response, _)| response)
        .map_err(Into::into)
    }

    pub(crate) fn bind_service_worker(
        &mut self,
        runtime: RendererBrowserContextRuntime,
        client_id: ServiceWorkerClientId,
    ) {
        let document_url = self.document_url();
        self.service_worker = Some(std::sync::Arc::new(
            move |request, resource_type, load, cancel| {
                let destination =
                    ServiceWorkerRequestDestination::for_subresource_resource_type(resource_type)?;
                if load.request_client().bypass_service_worker()
                    || !matches!(request.url.scheme(), "http" | "https")
                    || runtime
                        .service_worker_controller_for_fetch(client_id, &request.url)
                        .is_none()
                {
                    return None;
                }
                let runtime = runtime.clone();
                let document_url = document_url.clone();
                let load = load.clone();
                Some(Box::pin(async move {
                    runtime
                        .fetch_service_worker_subresource_for_client_with_metadata(
                            client_id,
                            document_url,
                            &request,
                            &load.request_client(),
                            load.task_runner(),
                            destination,
                            resource_type,
                            cancel,
                        )
                        .await
                }))
            },
        ));
    }

    pub(crate) async fn fetch_resource(
        &self,
        request: Request,
        resource_type: SubresourceResourceType,
        initiator: SubresourceRequestInitiatorType,
    ) -> Result<(NavigationResponse, ResourceResponseProvenance), ResourceResponseFailure> {
        let (load, network, started) = self
            .prepare_resource_request(&request, resource_type, initiator)
            .ok_or_else(|| {
                ResourceResponseFailure::Request("Document resource owner retired".into())
            })?;
        network.observe(started);
        self.fetch_started_resource(request, resource_type, load, network)
            .await
    }

    pub(crate) fn fetch_started_resource(
        &self,
        request: Request,
        resource_type: SubresourceResourceType,
        load: crate::network::loads::ResourceLoadLease,
        network: std::sync::Arc<crate::network::ResourceTransfer>,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<(NavigationResponse, ResourceResponseProvenance), ResourceResponseFailure>,
    > {
        Box::pin(async move {
            struct PendingLoad(crate::network::loads::ResourceLoadLease);
            impl Drop for PendingLoad {
                fn drop(&mut self) {
                    self.0.cancel();
                }
            }
            // A callback/cache producer can outlive this future. Its caller still
            // owns cancellation of this one consumer when the future is dropped.
            let _pending = PendingLoad(load.clone());
            let request = request.with_request_origin(self.fetch_context().request_origin());
            let client = load.request_client();
            let cancel = FetchCancelHandle::new();
            let controlled = self
                .service_worker
                .as_ref()
                .and_then(|fetch| fetch(request.clone(), resource_type, &load, cancel.clone()));

            load.attach_cancel_handle(cancel.clone());
            let (retired, cancelled) = tokio::sync::oneshot::channel();
            load.attach_consumer_cancel(move || {
                let _ = retired.send(());
            });
            let stream = ResourceResponseStream::for_load(network, &load, resource_type);
            let result = tokio::select! {
                biased;
                result = async {
                    let mut provenance = ResourceResponseProvenance::Network;
                    let controlled_response = match controlled {
                        Some(response) => response.await?,
                        None => None,
                    };
                    if resource_type == SubresourceResourceType::Script && controlled_response.is_none() {
                        // The declined controlled operation hands the same consumer
                        // to the shared script cache, including its cancellation hook.
                        load.release_consumer_cancel();
                        let (send, receive) = tokio::sync::oneshot::channel();
                        let network = stream.network.clone();
                        let terminal_network = network.clone();
                        client.fetch_cacheable_script_text_callback_with_load(
                            request, load.clone(), Some(network), move |result| {
                                terminal_network.complete(&result);
                                let _ = send.send(result);
                            },
                        )?;
                        return receive.await.map_err(|_| {
                            ResourceResponseFailure::Request("Script cache producer stopped".into())
                        })?.map(|response| (response.into(), ResourceResponseProvenance::Network));
                    }
                    let response = match controlled_response {
                        Some(response) => {
                            provenance = ResourceResponseProvenance::ServiceWorker { filter: response.response_filter };
                            *response.response
                        }
                        None => client.fetch_raw_stream_with_cancel_and_network_metadata(request, cancel).await?,
                    };
                    let body = stream.collect(response).await?;
                    body.publish(&stream.network, None);
                    Ok::<_, ResourceResponseFailure>((body.into_navigation_response()?, provenance))
                } => result,
                Ok(()) = cancelled => Err(stream.failure("Document resource load cancelled".into())),
            };
            if let Err(error) = &result {
                stream.network.failed(error);
            }
            load.finish();
            result
        })
    }
}
