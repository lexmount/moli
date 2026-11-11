//! Pending worker network requests, response snapshots and shared request helpers.

use super::*;

pub(in crate::worker) enum WorkerFetchClient {
    Promise {
        resolver: v8::Global<v8::PromiseResolver>,
        font_face: Option<v8::Global<v8::Object>>,
    },
    EventSource(v8::Global<v8::Object>),
}

impl WorkerFetchClient {
    pub(super) fn resolver(&self) -> &v8::Global<v8::PromiseResolver> {
        match self {
            Self::Promise { resolver, .. } => resolver,
            Self::EventSource(_) => {
                unreachable!("EventSource is delivered through networking tasks, not a Promise")
            }
        }
    }

    pub(super) fn font_face(&self) -> Option<&v8::Global<v8::Object>> {
        match self {
            Self::Promise { font_face, .. } => font_face.as_ref(),
            Self::EventSource(_) => None,
        }
    }

    pub(super) fn event_source(&self) -> Option<&v8::Global<v8::Object>> {
        match self {
            Self::EventSource(source) => Some(source),
            Self::Promise { .. } => None,
        }
    }
}

pub(in crate::worker) struct PendingWorkerFetch {
    pub(in crate::worker) upload: Option<crate::network_host::RequestUpload>,
    pub(super) timing: WorkerResourceTiming,
    pub(in crate::worker) kind: fetch::WorkerFetchKind,
    pub(in crate::worker) client: WorkerFetchClient,
    pub(in crate::worker) request_headers: moli_fetch::RequestHeaders,

    pub(in crate::worker) document_url: Url,
    /// Effective network credentials; the input Request and FetchEvent retain
    /// the author's original credentials mode.
    pub(in crate::worker) credentials_mode: RequestCredentialsMode,
    pub(in crate::worker) request_mode: moli_fetch::RequestMode,
    pub(in crate::worker) redirect_mode: RequestRedirectMode,

    pub(in crate::worker) policy_context: crate::types::SubresourcePolicyContext,
    pub(in crate::worker) resource_timing: crate::context_bootstrap::FetchResourceTiming,
    pub(in crate::worker) signal_id: Option<u32>,
    pub(in crate::worker) load: ResourceLoadLease,
    pub(in crate::worker) request_url: Url,
    pub(in crate::worker) request_method: String,

    pub(in crate::worker) streaming_body_source_id: Option<NetworkBodySourceId>,
    pub(in crate::worker) streaming_needs_orb_body_validation: bool,
}

pub(crate) enum WorkerFetchEvent {
    BrowserTimeout {
        timer_id: u32,
    },
    EventSourceState {
        fetch_id: u32,
        body_source_id: NetworkBodySourceId,
        last_event_id: String,
        reconnect_delay_ms: u64,
    },
    EventSourceMessage {
        fetch_id: u32,
        body_source_id: NetworkBodySourceId,
        message: crate::network_host::EventSourceMessage,
    },
    EventSourceFailure {
        fetch_id: u32,
        error: String,
        mode: crate::network_host::EventSourceTerminalMode,
    },

    RequestBodyProducer {
        fetch_id: u32,
        producer: crate::context_bootstrap::RequestBodyProducer,
    },
    ServiceWorkerUploadReady {
        event_id: crate::runtime::ServiceWorkerEventId,
        result: Result<(), moli_fetch::UploadBodyError>,
    },
    ServiceWorkerUploadFinished {
        event_id: crate::runtime::ServiceWorkerEventId,
        result: Result<(), moli_fetch::UploadBodyError>,
    },

    RequestUploadReady { fetch_id: u32, result: Result<(), moli_fetch::UploadBodyError> },
    RedirectAdmission { fetch_id: u32, admission: crate::network_host::RedirectAdmission },
    PolicyViolations(Vec<crate::content_security_policy::ContentSecurityPolicyUrlViolation>),



    Completion(Box<WorkerFetchCompletion>),
    StreamingStarted(WorkerFetchStreamingStarted),
    StreamingChunk(WorkerFetchStreamingChunk),
    StreamingFinished(WorkerFetchStreamingFinished),
    ResourceTiming {
        fetch_id: u32,
        timing: WorkerResourceTiming,
    },
}

pub(crate) struct WorkerFetchCompletion {
    pub(crate) fetch_id: u32,

    pub(crate) result: Result<WorkerFetchResponse, String>,
}

pub(crate) struct WorkerFetchStreamingStarted {
    pub(crate) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    pub(crate) skip_fetch_security_validation: bool,
    pub(crate) fetch_id: u32,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) head: ResponseHead,
    pub(crate) metadata: Arc<moli_fetch_types::FetchResponseMetadata>,
}

pub(crate) struct WorkerFetchStreamingChunk {
    pub(crate) fetch_id: u32,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct WorkerFetchStreamingFinished {
    pub(crate) fetch_id: u32,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) result: Result<crate::context_bootstrap::ResourcePerformanceEntry, String>,
}

pub(crate) struct WorkerFetchResponse {
    pub(crate) head: ResponseHead,
    pub(crate) body: ResponseBody,
    pub(crate) metadata: moli_fetch_types::FetchResponseMetadata,
    pub(crate) resource_timing: crate::context_bootstrap::ResourcePerformanceEntry,
}

pub(in crate::worker) struct PendingWorkerXhr {
    pub(super) timing: WorkerResourceTiming,
    pub(in crate::worker) xhr: v8::Global<v8::Object>,
    pub(in crate::worker) credentials_mode: RequestCredentialsMode,


    pub(in crate::worker) load: ResourceLoadLease,
}

pub(in crate::worker) struct WorkerXhrCompletion {
    pub(in crate::worker) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
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
    pub(in crate::worker) stream_abort: Option<crate::context_bootstrap::WebSocketStreamAbortRegistration>,
    pub(in crate::worker) wrapper: v8::Global<v8::Object>,
    pub(in crate::worker) connection: WebSocketConnectionHandle,
    pub(in crate::worker) document_url: Url,
    pub(in crate::worker) url: Url,
    pub(in crate::worker) load: Option<ResourceLoadLease>,
    pub(in crate::worker) handshake_recorded: bool,
}

pub(in crate::worker) enum WorkerXhrEvent {
    RedirectAdmission {
        xhr_id: u32,
        admission: crate::network_host::RedirectAdmission,
    },
    ResourceTiming {
        xhr_id: Option<u32>,
        timing: WorkerResourceTiming,
    },
    Upload {
        xhr_id: u32,
        report: moli_fetch::UploadReport,
    },
    Completion(Box<WorkerXhrCompletion>),
}
