use moli_fetch::{Response, ResponseHead};

use crate::{
    page_task_queue::RendererResourceCompletionSender,
    types::{
        AsyncSubresourceFetchEvent, AsyncSubresourceStreamingChunk,
        AsyncSubresourceStreamingStarted, NetworkBodySourceId, PendingSubresourceFetchInfo,
        SubresourceResponseBody,
    },
};

pub(crate) enum ControlledXhrBody {
    Buffered(Response),
    Streamed {
        head: ResponseHead,
        body: SubresourceResponseBody,
    },
}

impl ControlledXhrBody {
    pub(super) fn head(&self) -> ResponseHead {
        match self {
            Self::Buffered(response) => response.head(),
            Self::Streamed { head, .. } => head.clone(),
        }
    }
    pub(super) fn body(&self) -> SubresourceResponseBody {
        match self {
            Self::Buffered(response) => SubresourceResponseBody::from_fetch_response(response),
            Self::Streamed { body, .. } => body.clone(),
        }
    }
}

pub(crate) enum ControlledXhrCompletion {
    Buffered(Box<super::ControlledXhrResponse>),
    Streamed {
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
        result: Result<(), String>,
    },
}

/// The transport sends native networking tasks; only their ordinary owner
/// delivery can enter XHR callbacks. Body capture uses the existing disk pool.
pub(crate) struct ControlledXhrStream {
    completion: RendererResourceCompletionSender,
    pub(super) body_source_id: Option<NetworkBodySourceId>,
}

impl ControlledXhrStream {
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
    ) -> Result<(), String> {
        let body_source_id = crate::network_host::new_network_body_source_id();
        let started = AsyncSubresourceStreamingStarted {
            skip_fetch_security_validation: false,
            response_filter: None,
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
