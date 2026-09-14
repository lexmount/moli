use std::sync::Arc;

use crate::{
    network::{
        PausedResourceResponse, RendererResourceTaskRunner, ResourceBodyResponse,
        ResourceResponseBody, ResourceResponseConsumer, ResourceResponseFailure,
        ResourceResponseStream,
    },
    page_task_queue::RendererResourceCompletionSender,
    types::{
        AsyncSubresourceFetchCompletion, AsyncSubresourceFetchEvent,
        AsyncSubresourceStreamingChunk, AsyncSubresourceStreamingStarted, NetworkBodySourceId,
    },
};

/// Delivery back to the Page that admitted the request. The physical body and
/// its decision authority are shared with Worker consumers in network.
pub(crate) struct ResourceFetchReceiver {
    pub(crate) task_runner: RendererResourceTaskRunner,
    pub(crate) completion_tx: RendererResourceCompletionSender,
    pub(crate) internal_id: u64,
    pub(crate) request_url: url::Url,
    pub(crate) resource: Arc<ResourceResponseStream>,
    pub(crate) response_status_text: Option<String>,
    pub(crate) skip_fetch_security_validation: bool,
    pub(crate) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    pub(crate) network_error_text: Option<String>,
    stream_to_js: bool,
    body_source_id: Option<NetworkBodySourceId>,
}

impl ResourceFetchReceiver {
    pub(crate) fn new(
        task_runner: RendererResourceTaskRunner,
        completion_tx: RendererResourceCompletionSender,
        internal_id: u64,
        request_url: url::Url,
        resource: Arc<ResourceResponseStream>,
    ) -> Self {
        Self {
            task_runner,
            completion_tx,
            internal_id,
            request_url,
            resource,
            response_status_text: None,
            skip_fetch_security_validation: false,
            response_filter: None,
            network_error_text: None,
            stream_to_js: false,
            body_source_id: None,
        }
    }

    pub(crate) fn pause(
        mut self,
        body: Arc<ResourceResponseBody>,
        stream_to_js: bool,
    ) -> PausedResourceResponse {
        self.stream_to_js = stream_to_js;
        PausedResourceResponse::new(body, Box::new(self))
    }

    pub(crate) fn receive_or_pause(mut self, body: Arc<ResourceResponseBody>, stream_to_js: bool) {
        self.stream_to_js = stream_to_js;
        if self.resource.intercepts_response(&body.head()) {
            let internal_id = self.internal_id;
            let sender = self.completion_tx.clone();
            let response = self.pause(body, stream_to_js);
            let _ =
                sender.send_async_subresource_event(AsyncSubresourceFetchEvent::ResponsePaused {
                    internal_id,
                    response: Box::new(response),
                });
        } else {
            let runner = self.task_runner.clone();
            body.resume(Box::new(self));
            body.start(&runner);
        }
    }

    pub(crate) fn complete(
        self,
        body_source_id: Option<NetworkBodySourceId>,
        result: Result<ResourceBodyResponse, ResourceResponseFailure>,
    ) {
        let completion = AsyncSubresourceFetchCompletion {
            internal_id: self.internal_id,
            response_status_text: self.response_status_text,
            skip_fetch_security_validation: self.skip_fetch_security_validation,
            response_filter: self.response_filter,
            network_error_text: self.network_error_text,
            network_request_headers: self.resource.record_request_headers(None),
            result,
        };
        if let Some(body_source_id) = body_source_id {
            let _ = self.completion_tx.send_async_subresource_event(
                AsyncSubresourceFetchEvent::TransportStreamingFinished {
                    body_source_id,
                    completion: Box::new(super::CompletedResourceFetch::new(
                        self.resource,
                        completion,
                    )),
                },
            );
        } else {
            super::send_resource_completion(&self.completion_tx, self.resource, completion);
        }
    }
}

impl ResourceResponseConsumer for ResourceFetchReceiver {
    fn task_runner(&self) -> RendererResourceTaskRunner {
        self.task_runner.clone()
    }
    fn detach(&mut self) {
        self.stream_to_js = false;
    }
    fn response_started(&mut self, head: moli_fetch::ResponseHead) {
        if self.stream_to_js {
            let body_source_id = super::new_network_body_source_id();
            self.body_source_id = Some(body_source_id);
            let _ = self.completion_tx.send_async_subresource_event(
                AsyncSubresourceFetchEvent::StreamingStarted(Box::new(
                    AsyncSubresourceStreamingStarted {
                        skip_fetch_security_validation: self.skip_fetch_security_validation,
                        response_filter: self.response_filter,
                        internal_id: self.internal_id,
                        request_url: self.request_url.clone(),
                        body_source_id,
                        head,
                    },
                )),
            );
        }
    }
    fn data_received(&mut self, bytes: Vec<u8>) {
        if let Some(body_source_id) = self.body_source_id {
            let _ = self.completion_tx.send_async_subresource_event(
                AsyncSubresourceFetchEvent::StreamingChunk(AsyncSubresourceStreamingChunk {
                    body_source_id,
                    bytes,
                }),
            );
        }
    }
    fn complete(
        mut self: Box<Self>,
        result: Result<ResourceBodyResponse, ResourceResponseFailure>,
        network_error_text: Option<String>,
    ) {
        self.network_error_text = network_error_text.or(self.network_error_text.take());
        let id = self.body_source_id;
        (*self).complete(id, result);
    }
    fn discard(self: Box<Self>) {}
}
