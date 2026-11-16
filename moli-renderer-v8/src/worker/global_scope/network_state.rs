//! Pending worker network requests, response snapshots and shared request helpers.

use super::*;

pub(in crate::worker) struct PendingWorkerFetch {
    pub(in crate::worker) resolver: v8::Global<v8::PromiseResolver>,
    pub(in crate::worker) resource_timing: crate::context_bootstrap::FetchResourceTiming,
    pub(in crate::worker) document_url: Url,
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

/// One pending wake per response, sharing the native disk-backed capture.
/// A blocked Worker must not retain another Vec for every network chunk.
#[derive(Clone)]
pub(crate) struct WorkerFetchStreamingChunk {
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) body: crate::network::ResourceInterceptionBody,
    pending: Arc<parking_lot::Mutex<PendingBodyDelivery>>,
}

#[derive(Default)]
struct PendingBodyDelivery {
    offset: usize,
    end: usize,
    queued: bool,
}

impl WorkerFetchStreamingChunk {
    pub(crate) fn new(
        body_source_id: NetworkBodySourceId,
        body: crate::network::ResourceInterceptionBody,
    ) -> Self {
        Self {
            body_source_id,
            body,
            pending: Arc::new(parking_lot::Mutex::new(PendingBodyDelivery::default())),
        }
    }
    pub(crate) fn publish(&self, end: usize, sender: &mpsc::UnboundedSender<WorkerFetchEvent>) {
        let mut pending = self.pending.lock();
        pending.end = end;
        self.enqueue(&mut pending, sender);
    }
    fn enqueue(
        &self,
        pending: &mut PendingBodyDelivery,
        sender: &mpsc::UnboundedSender<WorkerFetchEvent>,
    ) {
        if pending.offset < pending.end && !pending.queued {
            pending.queued = true;
            let _ = sender.send(WorkerFetchEvent::StreamingChunk(self.clone()));
        }
    }
    pub(crate) fn requeue(&self, sender: &mpsc::UnboundedSender<WorkerFetchEvent>) {
        self.enqueue(&mut self.pending.lock(), sender);
    }
    pub(crate) fn has_pending(&self) -> bool {
        let pending = self.pending.lock();
        pending.offset < pending.end
    }
    pub(crate) fn discard(&self) {
        let mut pending = self.pending.lock();
        pending.offset = pending.end;
    }
    pub(crate) fn take_range(&self) -> std::ops::Range<usize> {
        let mut pending = self.pending.lock();
        let end = pending.offset.saturating_add(64 * 1024).min(pending.end);
        let range = pending.offset..end;
        pending.offset = end;
        pending.queued = false;
        range
    }
}

pub(crate) struct WorkerFetchStreamingFinished {
    pub(crate) fetch_id: u32,
    pub(crate) delivery: WorkerFetchStreamingChunk,
    pub(crate) body_source_id: NetworkBodySourceId,
    pub(crate) head: ResponseHead,
    pub(crate) result: Result<SubresourceResponseBody, String>,
}

impl WorkerFetchStreamingFinished {
    pub(crate) fn after_body_delivery(
        self,
        sender: &mpsc::UnboundedSender<WorkerFetchEvent>,
    ) -> Option<Self> {
        if self.result.is_ok() && self.delivery.has_pending() {
            self.delivery.requeue(sender);
            let _ = sender.send(WorkerFetchEvent::StreamingFinished(self));
            None
        } else {
            Some(self)
        }
    }
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
    pub(in crate::worker) document_url: Url,
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

#[cfg(test)]
mod body_delivery_tests {
    use super::*;

    #[test]
    fn coalesced_ranges_keep_bytes_in_the_native_capture_and_rearm_after_delivery() {
        let body = crate::network::ResourceInterceptionBody::from_complete(
            SubresourceResponseBody::from_bytes(b"abcdefgh".to_vec()),
        );
        let delivery = WorkerFetchStreamingChunk::new(
            crate::network_host::new_network_body_source_id(),
            body.clone(),
        );
        let (sender, mut receiver) = mpsc::unbounded_channel();
        delivery.publish(3, &sender);
        delivery.publish(5, &sender);
        let WorkerFetchEvent::StreamingChunk(first) = receiver.try_recv().unwrap() else {
            panic!("body wake")
        };
        assert!(
            receiver.try_recv().is_err(),
            "at most one wake per response"
        );
        assert_eq!(first.take_range(), 0..5);
        assert_eq!(first.body.read_captured_chunk(0, 5).unwrap(), b"abcde");
        // Data arriving after the Worker took its prefix queues a distinct
        // wake and never repeats bytes already handed to JS.
        delivery.publish(8, &sender);
        let WorkerFetchEvent::StreamingChunk(second) = receiver.try_recv().unwrap() else {
            panic!("next body wake")
        };
        assert_eq!(second.take_range(), 5..8);
        assert_eq!(second.body.read_captured_chunk(5, 3).unwrap(), b"fgh");
        assert!(receiver.try_recv().is_err());
        drop(receiver);
        delivery.publish(8, &sender);
        assert!(
            !body.cancellation_handle().is_cancelled(),
            "JS exit cannot cancel a keepalive transfer"
        );
    }
    #[test]
    fn pending_body_delivery_yields_and_defers_close_until_the_last_data_turn() {
        let bytes = vec![42; 3 * 64 * 1024];
        let capture = SubresourceResponseBody::from_bytes(bytes.clone());
        let delivery = WorkerFetchStreamingChunk::new(
            crate::network_host::new_network_body_source_id(),
            crate::network::ResourceInterceptionBody::from_complete(capture.clone()),
        );
        let (sender, mut receiver) = mpsc::unbounded_channel();
        delivery.publish(bytes.len(), &sender);
        sender
            .send(WorkerFetchEvent::StreamingFinished(
                WorkerFetchStreamingFinished {
                    fetch_id: 1,
                    body_source_id: delivery.body_source_id,
                    delivery: delivery.clone(),
                    head: ResponseHead {
                        final_url: "https://worker.test/body".parse().unwrap(),
                        status: 200,
                        status_text: None,
                        headers: Vec::new(),
                        request_cookie_report: None,
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        cache_state: Default::default(),
                        negotiated_http_version: None,
                    },
                    result: Ok(capture),
                },
            ))
            .unwrap_or_else(|_| panic!("live Worker queue"));
        // This stands for a runnable task/microtask after the first body turn.
        sender
            .send(WorkerFetchEvent::PolicyViolations(Vec::new()))
            .unwrap_or_else(|_| panic!("live Worker queue"));
        let mut delivered = Vec::new();
        let mut turns = 0;
        let mut finished = false;
        while let Ok(event) = receiver.try_recv() {
            match event {
                WorkerFetchEvent::StreamingChunk(chunk) => {
                    assert!(!finished);
                    let range = chunk.take_range();
                    assert_eq!(range.start, delivered.len());
                    assert!(range.len() <= 64 * 1024);
                    delivered.extend(
                        chunk
                            .body
                            .read_captured_chunk(range.start, range.len())
                            .unwrap(),
                    );
                    turns += 1;
                    chunk.requeue(&sender);
                }
                WorkerFetchEvent::PolicyViolations(_) => assert_eq!(
                    delivered.len(),
                    64 * 1024,
                    "other runnable work gets a turn before the remaining body"
                ),
                WorkerFetchEvent::StreamingFinished(completion) => {
                    if completion.after_body_delivery(&sender).is_some() {
                        assert_eq!(delivered, bytes);
                        assert!(!finished);
                        finished = true;
                    }
                }
                _ => panic!("body delivery test event"),
            }
        }
        assert!(finished);
        assert_eq!(turns, 3);
    }
}
