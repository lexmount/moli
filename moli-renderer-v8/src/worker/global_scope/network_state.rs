//! Pending worker network requests, response snapshots and shared request helpers.

use super::*;

pub(in crate::worker) struct PendingWorkerFetch {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
    pub(in crate::worker) resource_timing: crate::context_bootstrap::FetchResourceTiming,
    pub(in crate::worker) credentials_mode: RequestCredentialsMode,
    pub(in crate::worker) request_mode: moli_fetch::RequestMode,
    pub(in crate::worker) redirect_mode: RequestRedirectMode,

    pub(in crate::worker) policy_context: crate::types::SubresourcePolicyContext,
    pub(in crate::worker) signal_id: Option<u32>,
    pub(in crate::worker) load: ResourceLoadLease,
    pub(in crate::worker) request_url: Url,
    pub(in crate::worker) request_method: String,

    pub(in crate::worker) streaming_body_source_id: Option<NetworkBodySourceId>,
}

pub(crate) enum WorkerFetchEvent {
    PolicyViolations(Vec<crate::content_security_policy::ContentSecurityPolicyUrlViolation>),
    Completion(Box<WorkerFetchCompletion>),
    StreamingStarted(WorkerFetchStreamingStarted),
    StreamingChunk(WorkerFetchStreamingChunk),
    StreamingFinished(WorkerFetchStreamingFinished),
}

pub(crate) struct WorkerFetchCompletion {
    pub(crate) fetch_id: u32,

    pub(crate) result: Result<WorkerFetchResponse, String>,
}

pub(crate) struct WorkerFetchStreamingStarted {
    pub(crate) fetch_id: u32,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) head: ResponseHead,
}

pub(crate) struct WorkerFetchStreamingChunk {
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct WorkerFetchStreamingFinished {
    pub(crate) fetch_id: u32,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) head: ResponseHead,
    pub(crate) result: Result<SubresourceResponseBody, String>,
}

pub(crate) struct WorkerFetchResponse {
    pub(crate) response: Box<Response>,
    pub(crate) filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
}
impl WorkerFetchResponse {
    pub(super) fn response_filter(
        &self,
    ) -> Option<crate::types::AsyncSubresourceFetchResponseFilter> {
        self.filter.clone()
    }
    pub(super) fn head(&self) -> ResponseHead {
        self.response.head()
    }
    pub(super) fn subresource_response_body(&self) -> SubresourceResponseBody {
        SubresourceResponseBody::from_fetch_response(&self.response)
    }
    pub(super) fn into_fetch_parts(self) -> (ResponseHead, ResponseBody) {
        self.response.into_body()
    }
}

pub(in crate::worker) struct PendingWorkerXhr {
    pub(in crate::worker) xhr: v8::Global<v8::Object>,
    pub(in crate::worker) credentials_mode: RequestCredentialsMode,

    pub(in crate::worker) load: ResourceLoadLease,
}

pub(in crate::worker) struct WorkerXhrCompletion {
    pub(in crate::worker) csp_violations:
        Vec<crate::content_security_policy::ContentSecurityPolicyUrlViolation>,
    pub(in crate::worker) xhr_id: u32,
    // Rejections admitted before a native load use the Worker networking task.
    // Aborting or reopening before that task runs suppresses their delivery.
    pub(in crate::worker) local_request: Option<Box<PendingSubresourceFetchInfo>>,

    pub(in crate::worker) result: Result<Response, String>,
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
