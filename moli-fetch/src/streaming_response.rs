use std::{any::Any, collections::VecDeque, fmt};

use anyhow::{Result, anyhow};
use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::{
    FetchCancelHandle, NegotiatedHttpVersion, NetworkRequestExtraInfo, RawResponse, RedirectInfo,
    Response, ResponseBody, ResponseHead,
};

struct StreamingResponseLifetimeLease {
    _value: Box<dyn Any + Send + Sync>,
}

impl fmt::Debug for StreamingResponseLifetimeLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StreamingResponseLifetimeLease(..)")
    }
}

#[derive(Debug)]
pub struct StreamingHtmlResponse {
    pub final_url: Url,
    pub status: u16,
    pub status_text: Option<String>,
    pub headers: Vec<(String, Vec<u8>)>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<RedirectInfo>,
    pub from_cache: bool,
    pub cache_state: crate::ResponseCacheState,
    pub preload_state: crate::ResponsePreloadState,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_extra_info: Option<NetworkRequestExtraInfo>,
    body_chunks: mpsc::UnboundedReceiver<String>,
    cancel_handle: FetchCancelHandle,
    completion: Option<oneshot::Receiver<Result<()>>>,
}

impl StreamingHtmlResponse {
    /// Native observations from this response's selected transport attempt.
    pub fn response_timing(&self) -> crate::FetchResponseTiming {
        self.cancel_handle.response_timing()
    }

    pub fn new(
        final_url: Url,
        status: u16,
        headers: Vec<(String, Vec<u8>)>,
        body_chunks: mpsc::UnboundedReceiver<String>,
        cancel_handle: FetchCancelHandle,
        completion: oneshot::Receiver<Result<()>>,
    ) -> Self {
        Self::new_with_head(
            ResponseHead {
                status_text: None,
                final_url,
                status,
                headers,
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                cache_state: Default::default(),
                preload_state: Default::default(),
                negotiated_http_version: None,
            },
            body_chunks,
            cancel_handle,
            completion,
        )
    }

    pub fn new_with_head(
        head: ResponseHead,
        body_chunks: mpsc::UnboundedReceiver<String>,
        cancel_handle: FetchCancelHandle,
        completion: oneshot::Receiver<Result<()>>,
    ) -> Self {
        Self {
            final_url: head.final_url,
            status: head.status,
            status_text: head.status_text,
            headers: head.headers,
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            redirected: head.redirected,
            redirect_chain: head.redirect_chain,
            from_cache: head.from_cache,
            cache_state: head.cache_state,
            preload_state: head.preload_state.clone(),
            negotiated_http_version: head.negotiated_http_version,
            network_request_extra_info: None,
            body_chunks,
            cancel_handle,
            completion: Some(completion),
        }
    }

    pub fn head(&self) -> ResponseHead {
        ResponseHead {
            final_url: self.final_url.clone(),
            status: self.status,
            status_text: self.status_text.clone(),
            headers: self.headers.clone(),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.clone(),
            from_cache: self.from_cache,
            cache_state: self.cache_state,
            preload_state: self.preload_state.clone(),
            negotiated_http_version: self.negotiated_http_version,
        }
    }

    pub fn with_network_request_extra_info(
        mut self,
        network_request_extra_info: Option<NetworkRequestExtraInfo>,
    ) -> Self {
        self.network_request_extra_info = network_request_extra_info;
        self
    }

    pub fn network_request_extra_info(&self) -> Option<&NetworkRequestExtraInfo> {
        self.network_request_extra_info.as_ref()
    }

    pub fn into_body(self) -> (ResponseHead, ResponseBody) {
        let head = self.head();
        (head, ResponseBody::StreamingText(Box::new(self)))
    }

    pub async fn into_materialized_text_response(self) -> Result<Response> {
        let network_request_extra_info = self.network_request_extra_info.clone();
        let (head, body) = self.into_body();
        Response::from_head_and_body_source(head, body)
            .await
            .map(|response| response.with_network_request_extra_info(network_request_extra_info))
    }

    pub async fn next_chunk(&mut self) -> Option<String> {
        self.body_chunks.recv().await
    }

    pub fn try_next_chunk(&mut self) -> Option<String> {
        // Used by the phase-one streaming parser to coalesce chunks that are
        // already buffered. This must stay nonblocking so fetch backpressure
        // still comes from the response channel.
        self.body_chunks.try_recv().ok()
    }

    pub async fn finish(&mut self) -> Result<()> {
        while self.body_chunks.recv().await.is_some() {}
        let completion = self
            .completion
            .as_mut()
            .expect("streaming html completion should only be awaited once")
            .await;
        // Keep the receiver installed while it is pending so cancelling this
        // future leaves Drop armed to cancel the underlying transfer.
        self.completion = None;
        completion.map_err(|_| anyhow!("streaming html completion channel closed"))?
    }
}

impl Drop for StreamingHtmlResponse {
    fn drop(&mut self) {
        if self.completion.is_some() {
            self.cancel_handle.cancel();
        }
    }
}

#[derive(Debug, Default)]
struct NavigationBodyState {
    buffered_chunks: VecDeque<Vec<u8>>,
    mime_type: Option<String>,
}

struct BodyPrefixObserver {
    limit: usize,
    bytes: Vec<u8>,
    callback: Box<dyn FnOnce(Vec<u8>) + Send + Sync>,
}

impl std::fmt::Debug for BodyPrefixObserver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BodyPrefixObserver")
            .field("limit", &self.limit)
            .field("buffered", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct StreamingRawResponse {
    pub final_url: Url,
    pub status: u16,
    pub status_text: Option<String>,
    pub headers: Vec<(String, Vec<u8>)>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<RedirectInfo>,
    pub from_cache: bool,
    pub cache_state: crate::ResponseCacheState,
    pub preload_state: crate::ResponsePreloadState,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_extra_info: Option<NetworkRequestExtraInfo>,
    body_chunks: mpsc::UnboundedReceiver<Vec<u8>>,
    navigation: Option<Box<NavigationBodyState>>,
    cancel_handle: FetchCancelHandle,
    completion: Option<oneshot::Receiver<Result<()>>>,
    lifetime_lease: Option<StreamingResponseLifetimeLease>,
    body_prefix_observer: Option<Box<BodyPrefixObserver>>,
}

impl StreamingRawResponse {
    pub fn response_timing(&self) -> crate::FetchResponseTiming {
        self.cancel_handle.response_timing()
    }

    /// Shares observations with a tee's separate cancellation owner. This
    /// does not give that owner authority to cancel the source transfer.
    pub fn share_response_progress(&self, owner: &FetchCancelHandle) {
        owner.adopt_response_progress(&self.cancel_handle);
    }

    pub fn new(
        final_url: Url,
        status: u16,
        headers: Vec<(String, Vec<u8>)>,
        request_cookie_report: Option<StoredCookieQueryReport>,
        cookie_set_reports: Vec<StoredCookieSetReport>,
        redirected: bool,
        redirect_chain: Vec<RedirectInfo>,
        body_chunks: mpsc::UnboundedReceiver<Vec<u8>>,
        cancel_handle: FetchCancelHandle,
        completion: oneshot::Receiver<Result<()>>,
    ) -> Self {
        Self {
            final_url,
            status,
            status_text: None,
            headers,
            request_cookie_report,
            cookie_set_reports,
            redirected,
            redirect_chain,
            from_cache: false,
            cache_state: Default::default(),
            preload_state: Default::default(),
            negotiated_http_version: None,
            network_request_extra_info: None,
            body_chunks,
            navigation: None,
            cancel_handle,
            completion: Some(completion),
            lifetime_lease: None,
            body_prefix_observer: None,
        }
    }

    pub fn new_with_head(
        head: ResponseHead,
        body_chunks: mpsc::UnboundedReceiver<Vec<u8>>,
        cancel_handle: FetchCancelHandle,
        completion: oneshot::Receiver<Result<()>>,
    ) -> Self {
        Self {
            final_url: head.final_url,
            status: head.status,
            status_text: head.status_text,
            headers: head.headers,
            request_cookie_report: head.request_cookie_report,
            cookie_set_reports: head.cookie_set_reports,
            redirected: head.redirected,
            redirect_chain: head.redirect_chain,
            from_cache: head.from_cache,
            cache_state: head.cache_state,
            preload_state: head.preload_state.clone(),
            negotiated_http_version: head.negotiated_http_version,
            network_request_extra_info: None,
            body_chunks,
            navigation: None,
            cancel_handle,
            completion: Some(completion),
            lifetime_lease: None,
            body_prefix_observer: None,
        }
    }

    /// Retains an owning runtime lease until this response finishes or is
    /// dropped. Higher layers use this to keep the exact transport owner alive
    /// after response headers have been delivered.
    pub fn with_lifetime_lease<T>(mut self, lease: T) -> Self
    where
        T: Any + Send + Sync,
    {
        self.lifetime_lease = Some(StreamingResponseLifetimeLease {
            _value: Box::new(lease),
        });
        self
    }

    /// Observes at most `limit` bytes without consuming or rewriting chunks.
    /// The callback runs once when the prefix is full or body EOF is observed.
    /// Dropping the response before either boundary drops the callback.
    /// Replaces any previously installed, pending observer.
    pub fn with_body_prefix_observer(
        mut self,
        limit: usize,
        callback: impl FnOnce(Vec<u8>) + Send + Sync + 'static,
    ) -> Self {
        self.body_prefix_observer = None;
        if limit == 0 {
            callback(Vec::new());
        } else {
            self.body_prefix_observer = Some(Box::new(BodyPrefixObserver {
                limit,
                bytes: Vec::new(),
                callback: Box::new(callback),
            }));
        }
        self
    }

    fn observe_body_prefix(&mut self, chunk: Option<&[u8]>) {
        let Some(observer) = self.body_prefix_observer.as_mut() else {
            return;
        };
        if let Some(chunk) = chunk {
            let count = (observer.limit - observer.bytes.len()).min(chunk.len());
            observer.bytes.extend_from_slice(&chunk[..count]);
        }
        if chunk.is_none() || observer.bytes.len() == observer.limit {
            let observer = self.body_prefix_observer.take().unwrap();
            (observer.callback)(observer.bytes);
        }
    }

    pub async fn next_chunk(&mut self) -> Option<Vec<u8>> {
        let buffered = self
            .navigation
            .as_mut()
            .and_then(|navigation| navigation.buffered_chunks.pop_front());
        let chunk = match buffered {
            Some(chunk) => Some(chunk),
            None => self.body_chunks.recv().await,
        };
        self.observe_body_prefix(chunk.as_deref());
        chunk
    }

    /// Peeks the bounded navigation prefix without consuming response bytes.
    /// Header overrides are supplied by the navigation owner after Fetch's
    /// response-stage decision. Cancelling this future retains already read
    /// chunks, and ordinary consumers observe every byte exactly once.
    pub async fn sniff_navigation_mime_type(
        &mut self,
        headers: &[(String, Vec<u8>)],
    ) -> Result<String> {
        let mut sniffer =
            moli_web_mime::NavigationMimeSniffer::new(self.final_url.scheme(), headers);
        let navigation = self.navigation.get_or_insert_with(Default::default);
        navigation.mime_type = None;
        for chunk in &navigation.buffered_chunks {
            sniffer.push(chunk);
        }
        let (mime_type, eof) = loop {
            if let Some(mime) = sniffer.mime_type() {
                break (mime.to_owned(), false);
            }
            match self.body_chunks.recv().await {
                Some(chunk) => {
                    sniffer.push(&chunk);
                    if !chunk.is_empty() {
                        navigation.buffered_chunks.push_back(chunk);
                    }
                }
                None => break (sniffer.finish().to_owned(), true),
            }
        };
        if eof {
            self.finish_transfer().await?;
        }
        self.navigation
            .as_mut()
            .expect("initialized navigation prefix state")
            .mime_type = Some(mime_type.clone());
        Ok(mime_type)
    }

    pub fn navigation_mime_type(&self) -> Option<&str> {
        self.navigation
            .as_ref()
            .and_then(|navigation| navigation.mime_type.as_deref())
    }

    pub fn with_network_request_extra_info(
        mut self,
        network_request_extra_info: Option<NetworkRequestExtraInfo>,
    ) -> Self {
        self.network_request_extra_info = network_request_extra_info;
        self
    }

    pub fn network_request_extra_info(&self) -> Option<&NetworkRequestExtraInfo> {
        self.network_request_extra_info.as_ref()
    }

    /// Returns cancellation authority for this exact streaming transfer.
    /// Owners that move the response into another lifecycle boundary can use
    /// the clone to retire the transport without retaining the response body.
    pub fn cancellation_handle(&self) -> FetchCancelHandle {
        self.cancel_handle.clone()
    }

    pub fn try_next_chunk(&mut self) -> Option<Vec<u8>> {
        // See StreamingHtmlResponse::try_next_chunk.
        if let Some(chunk) = self
            .navigation
            .as_mut()
            .and_then(|navigation| navigation.buffered_chunks.pop_front())
        {
            self.observe_body_prefix(Some(&chunk));
            return Some(chunk);
        }
        match self.body_chunks.try_recv() {
            Ok(chunk) => {
                self.observe_body_prefix(Some(&chunk));
                Some(chunk)
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.observe_body_prefix(None);
                None
            }
            Err(mpsc::error::TryRecvError::Empty) => None,
        }
    }

    /// Returns whether the producer has closed the body channel and every
    /// published chunk has already been consumed.
    ///
    /// This is deliberately narrower than request completion: callers still
    /// use [`Self::finish`] to observe the terminal fetch result. It only lets
    /// a streaming consumer distinguish an open, temporarily empty body from
    /// an already exhausted one without waiting.
    pub fn body_chunk_stream_is_exhausted(&self) -> bool {
        self.navigation
            .as_ref()
            .is_none_or(|navigation| navigation.buffered_chunks.is_empty())
            && self.body_chunks.is_closed()
            && self.body_chunks.is_empty()
    }

    pub async fn finish(&mut self) -> Result<()> {
        while self.next_chunk().await.is_some() {}
        self.finish_transfer().await
    }

    async fn finish_transfer(&mut self) -> Result<()> {
        // A short sniffed response can already have reached terminal success
        // before its retained prefix is replayed to the document consumer.
        let Some(completion) = self.completion.as_mut() else {
            return Ok(());
        };
        let completion = completion.await;
        // See StreamingHtmlResponse::finish: taking the receiver before the
        // await would make a cancelled finish future look completed to Drop.
        self.completion = None;
        // The exact transport runtime only has to remain alive until its
        // terminal result is known. Releasing it here lets an outer owner reap
        // a replaced runtime even when the caller retains this response value.
        // A cancelled finish future never reaches this point, so Drop remains
        // responsible for cancellation and lease release in that case.
        self.lifetime_lease = None;
        completion.map_err(|_| anyhow!("streaming raw completion channel closed"))?
    }

    pub fn head(&self) -> ResponseHead {
        ResponseHead {
            final_url: self.final_url.clone(),
            status: self.status,
            status_text: self.status_text.clone(),
            headers: self.headers.clone(),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: self.redirected,
            redirect_chain: self.redirect_chain.clone(),
            from_cache: self.from_cache,
            cache_state: self.cache_state,
            preload_state: self.preload_state.clone(),
            negotiated_http_version: self.negotiated_http_version,
        }
    }

    pub fn into_body(self) -> (ResponseHead, ResponseBody) {
        let head = self.head();
        (head, ResponseBody::StreamingBytes(Box::new(self)))
    }

    /// Drains the streamed body and materializes the compatibility raw
    /// response.
    ///
    /// Prefer chunked consumption for large bodies. This helper exists for
    /// compatibility surfaces that still need a complete [`RawResponse`].
    pub async fn into_materialized_raw_response(self) -> Result<RawResponse> {
        let progress = self.cancel_handle.clone();
        let network_request_extra_info = self.network_request_extra_info.clone();
        let (head, body) = self.into_body();
        RawResponse::from_head_and_body_source(head, body)
            .await
            .map(|response| {
                response
                    .with_body_sizes(progress.response_timing().body_sizes())
                    .with_network_request_extra_info(network_request_extra_info)
            })
    }

    pub async fn into_lossy_materialized_text_response(self) -> Result<Response> {
        let network_request_extra_info = self.network_request_extra_info.clone();
        let (head, body) = self.into_body();
        Response::from_head_and_body_source(head, body)
            .await
            .map(|response| response.with_network_request_extra_info(network_request_extra_info))
    }
}

impl Drop for StreamingRawResponse {
    fn drop(&mut self) {
        if self.completion.is_some() {
            self.cancel_handle.cancel();
        }
        // Release the exact transport owner before dropping the completion
        // receiver makes stream termination observable to other tasks.
        self.lifetime_lease = None;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::Poll,
    };

    use super::*;

    fn sample_response_head() -> ResponseHead {
        ResponseHead {
            status_text: None,
            final_url: Url::parse("http://example.test/final").expect("test URL"),
            status: 203,
            headers: vec![("content-type".to_owned(), b"text/plain".to_vec())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            cache_state: Default::default(),
            preload_state: Default::default(),
            negotiated_http_version: None,
        }
    }

    struct CompletionOrderLease {
        completion_tx: Option<oneshot::Sender<Result<()>>>,
        dropped_before_completion_closed: Arc<AtomicBool>,
    }

    impl Drop for CompletionOrderLease {
        fn drop(&mut self) {
            let completion_tx = self
                .completion_tx
                .as_ref()
                .expect("test completion sender should remain installed");
            self.dropped_before_completion_closed
                .store(!completion_tx.is_closed(), Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn cancelling_html_finish_keeps_transfer_cancellation_armed() {
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        drop(body_tx);
        let (completion_tx, completion_rx) = oneshot::channel();
        let cancel_handle = FetchCancelHandle::new();
        let mut stream = StreamingHtmlResponse::new_with_head(
            sample_response_head(),
            body_rx,
            cancel_handle.clone(),
            completion_rx,
        );
        let mut finish = Box::pin(stream.finish());

        std::future::poll_fn(|cx| {
            assert!(
                finish.as_mut().poll(cx).is_pending(),
                "the unresolved transfer completion should keep finish pending"
            );
            Poll::Ready(())
        })
        .await;
        drop(finish);
        drop(stream);

        assert!(
            cancel_handle.is_cancelled(),
            "cancelling finish must leave Drop responsible for the transfer"
        );
        assert!(
            completion_tx.send(Ok(())).is_err(),
            "cancelling finish should release its completion receiver"
        );
    }

    #[tokio::test]
    async fn cancelling_raw_finish_keeps_transfer_cancellation_armed() {
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        drop(body_tx);
        let (completion_tx, completion_rx) = oneshot::channel();
        let cancel_handle = FetchCancelHandle::new();
        let mut stream = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            cancel_handle.clone(),
            completion_rx,
        );
        let mut finish = Box::pin(stream.finish());

        std::future::poll_fn(|cx| {
            assert!(
                finish.as_mut().poll(cx).is_pending(),
                "the unresolved transfer completion should keep finish pending"
            );
            Poll::Ready(())
        })
        .await;
        drop(finish);
        drop(stream);

        assert!(
            cancel_handle.is_cancelled(),
            "cancelling finish must leave Drop responsible for the transfer"
        );
        assert!(
            completion_tx.send(Ok(())).is_err(),
            "cancelling finish should release its completion receiver"
        );
    }

    #[test]
    fn streaming_raw_response_drops_lifetime_lease_before_completion_receiver() {
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        drop(body_tx);
        let (completion_tx, completion_rx) = oneshot::channel();
        let dropped_before_completion_closed = Arc::new(AtomicBool::new(false));
        let response = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            FetchCancelHandle::new(),
            completion_rx,
        )
        .with_lifetime_lease(CompletionOrderLease {
            completion_tx: Some(completion_tx),
            dropped_before_completion_closed: Arc::clone(&dropped_before_completion_closed),
        });

        drop(response);

        assert!(
            dropped_before_completion_closed.load(Ordering::SeqCst),
            "the exact transport owner must be released before completion receiver closure is observable"
        );
    }

    #[tokio::test]
    async fn raw_prefix_observation_preserves_chunks_and_stops_at_the_limit() {
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let (prefix_tx, prefix_rx) = std::sync::mpsc::channel();
        let mut response = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            FetchCancelHandle::new(),
            completion_rx,
        )
        .with_body_prefix_observer(5, move |bytes| prefix_tx.send(bytes).unwrap());
        body_tx.send(vec![0, 255]).unwrap();
        body_tx.send(vec![1, 2, 3, 4, 5]).unwrap();
        body_tx.send(vec![6]).unwrap();
        drop(body_tx);
        completion_tx.send(Ok(())).unwrap();

        assert_eq!(response.next_chunk().await, Some(vec![0, 255]));
        assert!(matches!(
            prefix_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert_eq!(response.try_next_chunk(), Some(vec![1, 2, 3, 4, 5]));
        assert_eq!(prefix_rx.try_recv().unwrap(), vec![0, 255, 1, 2, 3]);
        assert_eq!(response.next_chunk().await, Some(vec![6]));
        assert_eq!(response.next_chunk().await, None);
        response.finish().await.unwrap();
        assert!(matches!(
            prefix_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn navigation_sniff_replays_a_split_signature_without_duplicate_observation() {
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let (prefix_tx, prefix_rx) = std::sync::mpsc::channel();
        let mut response = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            FetchCancelHandle::new(),
            completion_rx,
        )
        .with_body_prefix_observer(8, move |bytes| prefix_tx.send(bytes).unwrap());
        let first = b"\x89PNG\r\n\x1a".to_vec();
        body_tx.send(first.clone()).unwrap();
        let mut sniff = Box::pin(response.sniff_navigation_mime_type(&[]));
        std::future::poll_fn(|cx| {
            assert!(sniff.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        body_tx.send(b"\n".to_vec()).unwrap();
        assert_eq!(sniff.await.unwrap(), "image/png");
        assert!(!response.body_chunk_stream_is_exhausted());
        assert!(matches!(
            prefix_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert_eq!(response.next_chunk().await, Some(first));
        assert_eq!(response.try_next_chunk(), Some(b"\n".to_vec()));
        assert_eq!(prefix_rx.try_recv().unwrap(), b"\x89PNG\r\n\x1a\n");
        body_tx.send(b"tail".to_vec()).unwrap();
        drop(body_tx);
        completion_tx.send(Ok(())).unwrap();
        assert_eq!(response.next_chunk().await, Some(b"tail".to_vec()));
        assert_eq!(response.next_chunk().await, None);
        response.finish().await.unwrap();
        assert_eq!(response.head().headers, sample_response_head().headers);
        assert_eq!(response.navigation_mime_type(), Some("image/png"));
        assert!(matches!(
            prefix_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn cancelling_navigation_sniff_retains_bytes_and_transfer_cancellation() {
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let cancel = FetchCancelHandle::new();
        let mut response = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            cancel.clone(),
            completion_rx,
        );
        body_tx.send(b"GIF89".to_vec()).unwrap();
        let mut sniff = Box::pin(response.sniff_navigation_mime_type(&[]));
        std::future::poll_fn(|cx| {
            assert!(sniff.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(sniff);
        assert_eq!(response.try_next_chunk(), Some(b"GIF89".to_vec()));
        assert!(!cancel.is_cancelled());
        drop(response);
        assert!(cancel.is_cancelled());
        assert!(completion_tx.send(Ok(())).is_err());
    }

    #[tokio::test]
    async fn navigation_sniff_preserves_short_eof_bytes_and_transfer_errors() {
        for fails in [false, true] {
            let (body_tx, body_rx) = mpsc::unbounded_channel();
            let (completion_tx, completion_rx) = oneshot::channel();
            let mut response = StreamingRawResponse::new_with_head(
                sample_response_head(),
                body_rx,
                FetchCancelHandle::new(),
                completion_rx,
            );
            body_tx.send(b"short text".to_vec()).unwrap();
            drop(body_tx);
            completion_tx
                .send(if fails {
                    Err(anyhow!("sniff transfer failed"))
                } else {
                    Ok(())
                })
                .unwrap();
            let result = response.sniff_navigation_mime_type(&[]).await;
            if fails {
                assert_eq!(result.unwrap_err().to_string(), "sniff transfer failed");
            } else {
                assert_eq!(result.unwrap(), "text/plain");
                assert!(!response.body_chunk_stream_is_exhausted());
                assert_eq!(response.next_chunk().await, Some(b"short text".to_vec()));
                response.finish().await.unwrap();
                assert!(response.body_chunk_stream_is_exhausted());
            }
        }
    }

    #[tokio::test]
    async fn raw_prefix_observation_distinguishes_pending_input_from_body_eof() {
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let (prefix_tx, prefix_rx) = std::sync::mpsc::channel();
        let mut response = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            FetchCancelHandle::new(),
            completion_rx,
        )
        .with_body_prefix_observer(1024, move |bytes| prefix_tx.send(bytes).unwrap());

        assert_eq!(response.try_next_chunk(), None);
        body_tx.send(b"short".to_vec()).unwrap();
        assert_eq!(response.try_next_chunk(), Some(b"short".to_vec()));
        assert_eq!(response.try_next_chunk(), None);
        assert!(matches!(
            prefix_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        drop(body_tx);
        assert_eq!(response.try_next_chunk(), None);
        assert_eq!(prefix_rx.try_recv().unwrap(), b"short");
        completion_tx.send(Ok(())).unwrap();
        response.finish().await.unwrap();
    }

    #[tokio::test]
    async fn raw_finish_observes_a_short_prefix_without_changing_transfer_errors() {
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = oneshot::channel();
        let (prefix_tx, prefix_rx) = std::sync::mpsc::channel();
        let mut response = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            FetchCancelHandle::new(),
            completion_rx,
        )
        .with_body_prefix_observer(1024, move |bytes| prefix_tx.send(bytes).unwrap());
        body_tx.send(b"partial".to_vec()).unwrap();
        drop(body_tx);
        completion_tx.send(Err(anyhow!("transfer failed"))).unwrap();

        assert_eq!(
            response.finish().await.unwrap_err().to_string(),
            "transfer failed"
        );
        assert_eq!(prefix_rx.try_recv().unwrap(), b"partial");
    }

    #[test]
    fn dropping_raw_prefix_observation_preserves_transfer_cancellation() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<StreamingRawResponse>();
        let (_body_tx, body_rx) = mpsc::unbounded_channel();
        let (_completion_tx, completion_rx) = oneshot::channel();
        let (prefix_tx, prefix_rx) = std::sync::mpsc::channel();
        let cancel = FetchCancelHandle::new();
        let response = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            cancel.clone(),
            completion_rx,
        )
        .with_body_prefix_observer(1024, move |bytes| prefix_tx.send(bytes).unwrap());

        drop(response);
        assert!(cancel.is_cancelled());
        assert!(matches!(
            prefix_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn empty_raw_prefix_runs_immediately_and_replaces_the_pending_observer() {
        let (_body_tx, body_rx) = mpsc::unbounded_channel();
        let (_completion_tx, completion_rx) = oneshot::channel();
        let (prefix_tx, prefix_rx) = std::sync::mpsc::channel();
        let (empty_tx, empty_rx) = std::sync::mpsc::channel();
        let response = StreamingRawResponse::new_with_head(
            sample_response_head(),
            body_rx,
            FetchCancelHandle::new(),
            completion_rx,
        )
        .with_body_prefix_observer(1024, move |bytes| prefix_tx.send(bytes).unwrap())
        .with_body_prefix_observer(0, move |bytes| empty_tx.send(bytes).unwrap());

        assert_eq!(empty_rx.try_recv().unwrap(), Vec::<u8>::new());
        assert!(matches!(
            prefix_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
        drop(response);
        assert!(matches!(
            empty_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }
}
