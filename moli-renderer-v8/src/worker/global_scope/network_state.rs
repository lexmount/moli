//! Pending worker network requests, response snapshots and shared request helpers.

use super::*;

pub(in crate::worker) struct PendingWorkerFetch {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
    pub(in crate::worker) document_url: Url,
    pub(in crate::worker) credentials_mode: RequestCredentialsMode,
    pub(in crate::worker) request_mode: moli_fetch::RequestMode,
    pub(in crate::worker) redirect_mode: RequestRedirectMode,
    pub(in crate::worker) request_priority: Option<moli_fetch::FetchPriorityHint>,
    pub(in crate::worker) request_metadata:
        crate::service_worker_runtime::ServiceWorkerFetchRequestMetadata,
    pub(in crate::worker) policy_context: crate::types::SubresourcePolicyContext,
    pub(in crate::worker) signal_id: Option<u32>,
    pub(in crate::worker) load: ResourceLoadLease,
    pub(in crate::worker) request_url: Url,
    pub(in crate::worker) request_method: String,
    pub(in crate::worker) request_headers: moli_fetch::RequestHeaders,
    pub(in crate::worker) request_body: Option<String>,
    pub(in crate::worker) network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub(in crate::worker) network_record: Option<PendingWorkerFetchNetworkRecord>,
    pub(in crate::worker) paused_response: Option<PausedWorkerSubresourceResponse>,
    pub(in crate::worker) streaming_body_source_id: Option<NetworkBodySourceId>,
}

pub(in crate::worker) enum WorkerFetchEvent {
    Completion(Box<WorkerFetchCompletion>),
    StreamingStarted(WorkerFetchStreamingStarted),
    StreamingChunk(WorkerFetchStreamingChunk),
    StreamingFinished(WorkerFetchStreamingFinished),
}

pub(in crate::worker) struct WorkerFetchCompletion {
    pub(super) fetch_id: u32,
    pub(super) network_request_headers: Option<Vec<(String, String)>>,
    pub(super) result: Result<WorkerFetchResponse, String>,
}

pub(in crate::worker) struct WorkerFetchStreamingStarted {
    pub(super) fetch_id: u32,
    pub(super) body_source_id: NetworkBodySourceId,
    pub(super) head: ResponseHead,
    pub(super) network_request_headers: Option<Vec<(String, String)>>,
}

pub(in crate::worker) struct WorkerFetchStreamingChunk {
    pub(super) body_source_id: NetworkBodySourceId,
    pub(super) bytes: Vec<u8>,
}

pub(in crate::worker) struct WorkerFetchStreamingFinished {
    pub(super) fetch_id: u32,
    pub(super) body_source_id: NetworkBodySourceId,
    pub(super) head: ResponseHead,
    pub(super) result: Result<SubresourceResponseBody, String>,
}

pub(in crate::worker) enum WorkerFetchResponse {
    Materialized(Box<Response>),
    Streamed {
        head: Box<ResponseHead>,
        body: SubresourceResponseBody,
    },
}

impl WorkerFetchResponse {
    pub(super) fn head(&self) -> ResponseHead {
        match self {
            Self::Materialized(response) => response.head(),
            Self::Streamed { head, .. } => head.as_ref().clone(),
        }
    }

    pub(super) fn subresource_response_body(&self) -> SubresourceResponseBody {
        match self {
            Self::Materialized(response) => SubresourceResponseBody::from_fetch_response(response),
            Self::Streamed { body, .. } => body.clone(),
        }
    }

    pub(super) fn into_fetch_parts(self) -> WorkerFetchResponseParts {
        match self {
            Self::Materialized(response) => {
                let (head, body) = response.into_body();
                WorkerFetchResponseParts::Materialized {
                    head,
                    body: Box::new(body),
                }
            }
            Self::Streamed { head, body } => {
                WorkerFetchResponseParts::Subresource { head: *head, body }
            }
        }
    }
}

pub(super) enum WorkerFetchResponseParts {
    Materialized {
        head: ResponseHead,
        body: Box<ResponseBody>,
    },
    Subresource {
        head: ResponseHead,
        body: SubresourceResponseBody,
    },
}

#[derive(Clone)]
pub(in crate::worker) struct PendingWorkerFetchNetworkRecord {
    pub(crate) redirect_headers: Option<moli_fetch::RequestHeaders>,
    pub(in crate::worker) internal_id: u64,
    pub(in crate::worker) url: Url,
    pub(in crate::worker) method: String,
    pub(in crate::worker) request_headers: moli_fetch::RequestHeaders,
    pub(in crate::worker) request_body: Option<String>,
    pub(in crate::worker) initial_network_request_headers: Option<Vec<(String, String)>>,
    pub(in crate::worker) intercept_response: bool,
    pub(in crate::worker) handle_auth_requests: bool,
}

impl PendingWorkerFetchNetworkRecord {
    pub(super) fn follow_redirects(&mut self, head: &ResponseHead) {
        if head.redirect_chain.is_empty() {
            return;
        }
        let mut request = Request::get_with_url(self.url.clone())
            .with_redirect_headers(self.redirect_headers.take());
        request.method = std::mem::take(&mut self.method);
        request.body = self.request_body.clone().map(String::into_bytes);
        request.request_headers = std::mem::take(&mut self.request_headers);
        for redirect in &head.redirect_chain {
            request.apply_redirect_status(redirect.status);
        }
        self.url = head.final_url.clone();
        self.method = request.method;
        self.request_headers = request.request_headers;
        if request.body.is_none() {
            self.request_body = None;
        }
    }
}

pub(in crate::worker) struct PendingWorkerXhr {
    pub(in crate::worker) xhr: v8::Global<v8::Object>,
    pub(in crate::worker) document_url: Url,
    pub(in crate::worker) credentials_mode: RequestCredentialsMode,
    pub(in crate::worker) load: ResourceLoadLease,
    pub(in crate::worker) request_paused: bool,
    pub(in crate::worker) request_url: Url,
    pub(in crate::worker) request_method: String,
    pub(in crate::worker) request_headers: moli_fetch::RequestHeaders,
    pub(in crate::worker) request_body: Option<String>,
    pub(in crate::worker) network_request_handle: Option<SubresourceNetworkRequestHandle>,
    pub(in crate::worker) network_record: Option<PendingWorkerFetchNetworkRecord>,
    pub(in crate::worker) paused_response: Option<PausedWorkerSubresourceResponse>,
}

pub(in crate::worker) struct PendingWorkerCspReport {
    pub(in crate::worker) load: ResourceLoadLease,
    pub(in crate::worker) document_url: Url,
    pub(in crate::worker) request: Request,
    pub(in crate::worker) request_body: Option<String>,
    pub(in crate::worker) policy_context: crate::types::SubresourcePolicyContext,
    pub(in crate::worker) service_worker_runtime:
        Option<crate::service_worker_runtime::ServiceWorkerRuntimeService>,
    pub(in crate::worker) service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
}

pub(in crate::worker) struct PausedWorkerSubresourceResponse {
    pub(in crate::worker) head: ResponseHead,
    pub(in crate::worker) body: SubresourceResponseBody,
}

pub(in crate::worker) struct WorkerXhrCompletion {
    pub(in crate::worker) xhr_id: u32,
    pub(in crate::worker) network_request_headers: Option<Vec<(String, String)>>,
    pub(in crate::worker) result: Result<WorkerXhrResponse, String>,
}

pub(in crate::worker) enum WorkerXhrResponse {
    Materialized(Box<Response>),
    Streamed {
        head: Box<ResponseHead>,
        body: SubresourceResponseBody,
    },
}

impl WorkerXhrResponse {
    pub(super) fn head(&self) -> ResponseHead {
        match self {
            Self::Materialized(response) => response.head(),
            Self::Streamed { head, .. } => head.as_ref().clone(),
        }
    }

    pub(super) fn subresource_response_body(&self) -> SubresourceResponseBody {
        match self {
            Self::Materialized(response) => SubresourceResponseBody::from_fetch_response(response),
            Self::Streamed { body, .. } => body.clone(),
        }
    }

    pub(super) fn into_body_source(self) -> Result<(ResponseHead, ResponseBody), String> {
        match self {
            Self::Materialized(response) => Ok(response.into_body()),
            Self::Streamed { head, body } => body
                .materialize_bytes()
                .map(|bytes| (*head, ResponseBody::materialized_bytes(bytes)))
                .map_err(|error| format!("failed to materialize worker XHR body: {error}")),
        }
    }
}

pub(in crate::worker) struct WorkerWebSocketState {
    pub(in crate::worker) wrapper: v8::Global<v8::Object>,
    pub(in crate::worker) connection: WebSocketConnectionHandle,
    pub(in crate::worker) document_url: Url,
    pub(in crate::worker) url: Url,
    pub(in crate::worker) loader: crate::network::context::WorkerResourceLoader,
    pub(in crate::worker) load: Option<ResourceLoadLease>,
    pub(in crate::worker) opened: bool,
    pub(in crate::worker) network_recorded: bool,
}

pub(super) fn next_fetch_id(state: &mut WorkerGlobalState) -> u32 {
    state.next_fetch_id = state
        .next_fetch_id
        .checked_add(1)
        .expect("worker fetch id space exhausted");
    state.next_fetch_id
}

pub(super) fn next_xhr_id(state: &mut WorkerGlobalState) -> u32 {
    state.next_xhr_id = state
        .next_xhr_id
        .checked_add(1)
        .expect("worker XHR id space exhausted");
    state.next_xhr_id
}

pub(super) fn next_websocket_id(state: &mut WorkerGlobalState) -> u64 {
    state.next_websocket_id = state
        .next_websocket_id
        .checked_add(1)
        .expect("worker WebSocket id space exhausted");
    state.next_websocket_id
}

pub(super) fn worker_url_blocked(patterns: &[String], url: &Url) -> bool {
    let value = url.as_str();
    patterns
        .iter()
        .any(|pattern| moli_fetch::url_pattern_matches(pattern, value))
}

pub(super) fn merge_worker_request_headers(
    context_headers: &moli_fetch::RequestHeaders,
    request_headers: &moli_fetch::RequestHeaders,
) -> moli_fetch::RequestHeaders {
    let mut headers = context_headers.clone();
    headers.overlay(request_headers.clone());
    headers
}
