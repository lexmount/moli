use std::{fmt, sync::Arc};

use moli_fetch::{Response, ResponseHead};
use moli_page_types::SubresourceResponseBody;

#[derive(Clone, Debug)]
pub(crate) struct ResourceResponseHead {
    pub(crate) head: ResponseHead,
    pub(crate) network_request_headers: Option<Vec<(String, String)>>,
}

/// A failed stream still owns the response facts already received. Cache
/// consumers admitted near completion must not lose its head or partial body.
#[derive(Clone, Debug)]
pub(crate) enum ResourceResponseFailure {
    Request(String),
    PartialBody {
        message: String,
        response: Arc<ResourceResponseHead>,
        body: SubresourceResponseBody,
    },
}

impl fmt::Display for ResourceResponseFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(message) | Self::PartialBody { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for ResourceResponseFailure {}

impl From<String> for ResourceResponseFailure {
    fn from(message: String) -> Self {
        Self::Request(message)
    }
}

impl From<anyhow::Error> for ResourceResponseFailure {
    fn from(error: anyhow::Error) -> Self {
        Self::Request(format!("{error:#}"))
    }
}

pub(crate) type ResourceResponseResult = Result<Response, ResourceResponseFailure>;

/// Synchronous native fact publication only: implementations must not execute
/// script, invoke completion callbacks, or re-enter the resource cache. This
/// lets cache admission replay its current progress before a later chunk wins.
pub(crate) trait ResourceResponseObserver: Send + Sync {
    fn response_started(&self, response: Arc<ResourceResponseHead>);
    fn data_received(&self, bytes: usize);
}

/// Preserve the physical response, including the received prefix on failure.
pub(crate) async fn collect_observed_response(
    observed: moli_fetch::NetworkFetchResult<moli_fetch::StreamingRawResponse>,
    observer: Option<&dyn ResourceResponseObserver>,
) -> ResourceResponseResult {
    let (mut response, request_observation) = observed.into_parts();
    let head = Arc::new(ResourceResponseHead {
        head: response.head(),
        network_request_headers: request_observation.map(|request| request.into_headers()),
    });
    if let Some(observer) = observer {
        observer.response_started(head.clone());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        bytes.extend_from_slice(&chunk);
        if let Some(observer) = observer {
            observer.data_received(chunk.len());
        }
    }
    if let Err(error) = response.finish().await {
        return Err(ResourceResponseFailure::PartialBody {
            message: format!("{error:#}"),
            response: head,
            body: moli_page_types::SubresourceResponseBody::from_bytes(bytes),
        });
    }
    Ok(
        moli_fetch::RawResponse::from_head_and_body(head.head.clone(), bytes)
            .into_lossy_materialized_text_response(),
    )
}
