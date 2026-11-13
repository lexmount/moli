//! Native raced response replay. Header selection does not wait for EOF.

use std::sync::Arc;

use bytes::Bytes;
use moli_fetch::{FetchCancelHandle, ResponseHead};
use parking_lot::Mutex;
use tokio::sync::Notify;

pub(crate) enum RouterRaceCandidate {
    Network,
    Handler(crate::service_worker_runtime::ServiceWorkerDirectFetchResult),
    HandlerNetwork,
}

pub(crate) struct RouterRaceCompletionTarget {
    pub(crate) sender: tokio::sync::mpsc::UnboundedSender<RouterRaceCandidate>,
    pub(crate) response: Arc<RouterRaceResponse>,
    pub(crate) runner: crate::network::RendererResourceTaskRunner,
    pub(crate) cancel: FetchCancelHandle,
}

impl std::fmt::Debug for RouterRaceCompletionTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RouterRaceCompletionTarget")
            .finish_non_exhaustive()
    }
}

impl RouterRaceCompletionTarget {
    pub(crate) fn send(
        self,
        result: crate::service_worker_runtime::ServiceWorkerDirectFetchResult,
    ) -> bool {
        use crate::service_worker_runtime::ServiceWorkerDirectFetchResult;
        if matches!(
            result,
            ServiceWorkerDirectFetchResult::Fallback
                | ServiceWorkerDirectFetchResult::NetworkFallback { .. }
        ) {
            // A handler that did not respond uses the already running fetch.
            // Its candidate becomes ready at the network head, including 404s.
            let reader = self.response.network_reader();
            self.runner.spawn(async move {
                let candidate = match reader.head(&self.cancel).await {
                    Ok(_) => RouterRaceCandidate::HandlerNetwork,
                    Err(message) => RouterRaceCandidate::Handler(
                        ServiceWorkerDirectFetchResult::Failure(message),
                    ),
                };
                let _ = self.sender.send(candidate);
            });
            true
        } else {
            // Both producers append to one queue at response availability.
            // Polling two completed oneshots must not reorder the winner.
            self.sender
                .send(RouterRaceCandidate::Handler(result))
                .is_ok()
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RouterRaceResponseHead {
    pub(crate) head: ResponseHead,
    pub(crate) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    pub(crate) network_request_headers: Option<Vec<(String, String)>>,
}

pub(crate) struct RouterRaceEntry {
    pub(crate) owner: crate::service_worker_runtime::ServiceWorkerRunOwner,
    pub(crate) response: Arc<RouterRaceResponse>,
    pub(crate) request: crate::service_worker_runtime::ServiceWorkerFetchRequest,
}

impl RouterRaceEntry {
    pub(crate) fn matches_request(
        &self,
        request: &crate::service_worker_runtime::ServiceWorkerFetchRequest,
    ) -> bool {
        let original = &self.request;
        let headers = |values: &[(String, String)]| {
            let mut values = values
                .iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                .collect::<Vec<_>>();
            values.sort();
            values
        };
        original.url == request.url
            && original.method == request.method
            && original.body == request.body
            && headers(&original.headers) == headers(&request.headers)
            && original.request_mode == request.request_mode
            && original.credentials_mode == request.credentials_mode
            && original.redirect_mode == request.redirect_mode
            && original.priority.unwrap_or_default() == request.priority.unwrap_or_default()
            && original.metadata.cache == request.metadata.cache
            && original.metadata.referrer == request.metadata.referrer
            && original.metadata.referrer_policy == request.metadata.referrer_policy
            && original.metadata.integrity == request.metadata.integrity
            && original.metadata.keepalive == request.metadata.keepalive
    }
}

struct ResponseState {
    accepting_lookups: bool,
    network_route_live: bool,
    worker_readers: usize,
    head: Option<Result<Arc<RouterRaceResponseHead>, String>>,
    chunks: Vec<Bytes>,
    finished: Option<Result<(), String>>,
}

/// The map and the network producer own this native state. Request wrappers
/// carry a private event identity, never a persistent V8 root or a URL key.
pub(crate) struct RouterRaceResponse {
    state: Mutex<ResponseState>,
    changed: Notify,
    network_cancel: FetchCancelHandle,
}

impl RouterRaceResponse {
    pub(crate) fn new(network_cancel: FetchCancelHandle) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ResponseState {
                accepting_lookups: true,
                network_route_live: true,
                worker_readers: 0,
                head: None,
                chunks: Vec::new(),
                finished: None,
            }),
            changed: Notify::new(),
            network_cancel,
        })
    }

    pub(crate) fn network_cancel_handle(&self) -> FetchCancelHandle {
        self.network_cancel.clone()
    }

    /// Admit the lookup synchronously, before the worker queues its IO task.
    /// A handler response cannot cancel an already owned response clone.
    pub(crate) fn lookup(self: &Arc<Self>) -> Option<RouterRaceResponseReader> {
        let mut state = self.state.lock();
        if !state.accepting_lookups {
            return None;
        }
        state.worker_readers += 1;
        Some(RouterRaceResponseReader {
            response: self.clone(),
            cursor: 0,
            worker_owned: true,
        })
    }

    pub(crate) fn network_reader(self: &Arc<Self>) -> RouterRaceResponseReader {
        RouterRaceResponseReader {
            response: self.clone(),
            cursor: 0,
            worker_owned: false,
        }
    }

    pub(crate) fn close_lookups(&self) {
        self.state.lock().accepting_lookups = false;
    }

    /// Release only the competing network route. An admitted worker fetch owns
    /// a clone of that response, so its body remains live until it is dropped.
    pub(crate) fn handler_won(&self) {
        let cancel = {
            let mut state = self.state.lock();
            state.network_route_live = false;
            state.worker_readers == 0 && state.finished.is_none()
        };
        if cancel {
            self.network_cancel.cancel();
        }
    }

    pub(crate) fn publish_head(&self, head: RouterRaceResponseHead) {
        let mut state = self.state.lock();
        if state.head.is_none() {
            state.head = Some(Ok(Arc::new(head)));
            drop(state);
            self.changed.notify_waiters();
        }
    }

    pub(crate) fn push_chunk(&self, bytes: Bytes) {
        if bytes.is_empty() {
            return;
        }
        let mut state = self.state.lock();
        if state.finished.is_none() {
            state.chunks.push(bytes);
            drop(state);
            self.changed.notify_waiters();
        }
    }

    pub(crate) fn finish(&self, result: Result<(), String>) {
        let mut state = self.state.lock();
        if state.finished.is_some() {
            return;
        }
        if state.head.is_none() {
            state.head = Some(Err(result.clone().err().unwrap_or_else(|| {
                "Raced network transfer ended before response headers".to_owned()
            })));
        }
        state.finished = Some(result);
        drop(state);
        self.changed.notify_waiters();
    }
}

pub(crate) struct RouterRaceResponseReader {
    response: Arc<RouterRaceResponse>,
    cursor: usize,
    worker_owned: bool,
}

impl RouterRaceResponseReader {
    pub(crate) async fn head(
        &self,
        consumer_cancel: &FetchCancelHandle,
    ) -> Result<Arc<RouterRaceResponseHead>, String> {
        loop {
            if consumer_cancel.is_cancelled() {
                return Err("Raced fetch was aborted".to_owned());
            }
            let changed = self.response.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(head) = self.response.state.lock().head.clone() {
                return head;
            }
            tokio::select! {
                () = consumer_cancel.cancelled() => return Err("Raced fetch was aborted".to_owned()),
                () = self.response.network_cancel.cancelled() => return Err("Raced network transfer was aborted".to_owned()),
                () = changed => {}
            }
        }
    }

    pub(crate) async fn next_chunk(
        &mut self,
        consumer_cancel: &FetchCancelHandle,
    ) -> Result<Option<Bytes>, String> {
        loop {
            if consumer_cancel.is_cancelled() {
                return Err("Raced fetch was aborted".to_owned());
            }
            let changed = self.response.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self.response.state.lock();
                if let Some(Err(error)) = &state.finished {
                    return Err(error.clone());
                }
                if let Some(chunk) = state.chunks.get(self.cursor) {
                    self.cursor += 1;
                    return Ok(Some(chunk.clone()));
                }
                if let Some(result) = &state.finished {
                    return result.clone().map(|()| None);
                }
            }
            tokio::select! {
                () = consumer_cancel.cancelled() => return Err("Raced fetch was aborted".to_owned()),
                () = self.response.network_cancel.cancelled() => return Err("Raced network transfer was aborted".to_owned()),
                () = changed => {}
            }
        }
    }
}

impl Drop for RouterRaceResponseReader {
    fn drop(&mut self) {
        if !self.worker_owned {
            return;
        }
        let cancel = {
            let mut state = self.response.state.lock();
            state.worker_readers -= 1;
            !state.network_route_live && state.worker_readers == 0 && state.finished.is_none()
        };
        if cancel {
            self.response.network_cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn response_headers_are_available_before_the_body_finishes() {
        let parent = FetchCancelHandle::new();
        let response = RouterRaceResponse::new(parent.child_for_subrequest());
        let mut reader = response.network_reader();
        response.publish_head(RouterRaceResponseHead {
            head: ResponseHead {
                status_text: Some("OK".to_owned()),
                final_url: url::Url::parse("https://example.test/race").unwrap(),
                status: 200,
                headers: Vec::new(),
                request_cookie_report: None,
                request_include_credentials: false,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                cache_state: Default::default(),
                preload_state: Default::default(),
                negotiated_http_version: None,
            },
            response_filter: None,
            network_request_headers: None,
        });
        let head = tokio::time::timeout(std::time::Duration::from_secs(1), reader.head(&parent))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(head.head.status, 200);
        assert!(response.state.lock().finished.is_none());
        response.push_chunk(Bytes::from_static(b"later body"));
        response.finish(Ok(()));
        assert_eq!(
            reader.next_chunk(&parent).await.unwrap().unwrap(),
            "later body"
        );
        assert!(reader.next_chunk(&parent).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cancellation_rejects_even_when_successful_body_bytes_are_already_queued() {
        let parent = FetchCancelHandle::new();
        let response = RouterRaceResponse::new(parent.child_for_subrequest());
        let mut reader = response.network_reader();
        response.push_chunk(Bytes::from_static(b"unread"));
        response.finish(Ok(()));
        parent.cancel();
        assert!(reader.next_chunk(&parent).await.is_err());
    }

    #[tokio::test]
    async fn clones_replay_chunks_from_their_own_cursor_and_share_terminal_errors() {
        let parent = FetchCancelHandle::new();
        let response = RouterRaceResponse::new(parent.child_for_fetch_hop());
        let mut first = response.lookup().unwrap();
        response.push_chunk(Bytes::from_static(b"first"));
        assert_eq!(first.next_chunk(&parent).await.unwrap().unwrap(), "first");
        let mut second = response.lookup().unwrap();
        assert_eq!(second.next_chunk(&parent).await.unwrap().unwrap(), "first");
        response.push_chunk(Bytes::from_static(b"second"));
        for reader in [&mut first, &mut second] {
            assert_eq!(reader.next_chunk(&parent).await.unwrap().unwrap(), "second");
        }
        response.push_chunk(Bytes::from_static(b"discarded when the stream errors"));
        response.finish(Err("body transfer failed".to_owned()));
        for reader in [&mut first, &mut second] {
            assert_eq!(
                reader.next_chunk(&parent).await.unwrap_err(),
                "body transfer failed"
            );
        }
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn handler_winner_keeps_owned_worker_body_and_never_cancels_the_client_authority() {
        let parent = FetchCancelHandle::new();
        let response = RouterRaceResponse::new(parent.child_for_fetch_hop());
        let reader = response.lookup().unwrap();
        response.close_lookups();
        response.handler_won();
        assert!(response.lookup().is_none());
        assert!(!response.network_cancel_handle().is_cancelled());
        drop(reader);
        assert!(response.network_cancel_handle().is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn handler_winner_without_a_worker_reader_cancels_only_the_network_branch() {
        let parent = FetchCancelHandle::new();
        let response = RouterRaceResponse::new(parent.child_for_fetch_hop());
        response.close_lookups();
        response.handler_won();
        assert!(response.network_cancel_handle().is_cancelled());
        assert!(!parent.is_cancelled());
    }
}
