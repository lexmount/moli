use moli_fetch::ResponseHead;

use crate::{
    page_task_queue::RendererResourceCompletionSender,
    types::{
        AsyncSubresourceFetchEvent, AsyncSubresourceStreamingChunk,
        AsyncSubresourceStreamingStarted, NetworkBodySourceId, PendingSubresourceFetchInfo,
    },
};

pub(crate) enum ControlledXhrCompletion {
    Buffered(Box<super::XhrLoadResult>),
    Streamed {
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
        result: Result<(), String>,
    },
}

/// The transport sends native networking tasks; only their ordinary owner
/// delivery can enter XHR callbacks. Body capture uses the existing disk pool.
pub(crate) struct DocumentResourceStream {
    completion: RendererResourceCompletionSender,
    pub(super) body_source_id: Option<NetworkBodySourceId>,
}

impl DocumentResourceStream {
    pub(crate) fn new(completion: RendererResourceCompletionSender) -> Self {
        Self {
            completion,
            body_source_id: None,
        }
    }
    pub(crate) fn start(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        head: ResponseHead,
        network_request_headers: Option<Vec<(String, String)>>,
        body: crate::network::ResourceInterceptionBody,
        response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
        skip_fetch_security_validation: bool,
    ) -> Result<(), String> {
        let body_source_id = crate::network_host::new_network_body_source_id();
        let started = AsyncSubresourceStreamingStarted {
            skip_fetch_security_validation,
            response_filter,
            internal_id: info.internal_id,
            request_url: info.url.clone(),
            request_method: info.method.clone(),
            request_headers: info.request_headers.clone(),
            request_body: info.request_body.clone(),
            body_source_id,
            head,
            network_request_headers,
        };
        self.completion
            .send_async_subresource_event(
                AsyncSubresourceFetchEvent::InterceptedXhrStreamingStarted {
                    info: Box::new(info.clone()),
                    started: Box::new(started),
                    body,
                },
            )
            .map_err(|_| "XHR stream delivery owner is closed".to_owned())?;
        self.body_source_id = Some(body_source_id);
        Ok(())
    }
    pub(crate) fn chunk(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.completion
            .send_async_subresource_event(AsyncSubresourceFetchEvent::StreamingChunk(
                AsyncSubresourceStreamingChunk {
                    body_source_id: self
                        .body_source_id
                        .expect("stream must start before chunks"),
                    bytes,
                },
            ))
            .map_err(|_| "XHR stream delivery owner is closed".to_owned())
    }
}

pub(crate) enum ControlledResourceStream {
    Document(DocumentResourceStream),
    WorkerFetch(Box<super::fetch::ControlledFetchStream>),
}

impl ControlledResourceStream {
    pub(super) fn policy_violations(
        &self,
        violations: Vec<crate::content_security_policy::ContentSecurityPolicyUrlViolation>,
    ) {
        if let Self::WorkerFetch(stream) = self {
            stream.policy_violations(violations);
        }
    }

    pub(crate) fn new(completion: RendererResourceCompletionSender) -> Self {
        Self::Document(DocumentResourceStream::new(completion))
    }
    pub(super) fn body_source_id(&self) -> Option<NetworkBodySourceId> {
        match self {
            Self::Document(stream) => stream.body_source_id,
            Self::WorkerFetch(stream) => stream.body_source_id,
        }
    }
    pub(super) fn start(
        &mut self,
        info: &PendingSubresourceFetchInfo,
        head: ResponseHead,
        headers: Option<Vec<(String, String)>>,
        body: crate::network::ResourceInterceptionBody,
        filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
        skip_security: bool,
    ) -> Result<(), String> {
        match self {
            Self::Document(stream) => {
                stream.start(info, head, headers, body, filter, skip_security)
            }
            Self::WorkerFetch(stream) => {
                stream.set_response_filter(filter);
                stream.start(info, head, headers, body)
            }
        }
    }
    pub(super) fn chunk(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        match self {
            Self::Document(stream) => stream.chunk(bytes),
            Self::WorkerFetch(stream) => stream.chunk(bytes),
        }
    }
}
