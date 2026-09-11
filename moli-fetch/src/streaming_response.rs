use std::{any::Any, fmt};

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
    pub headers: Vec<(String, String)>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<RedirectInfo>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_extra_info: Option<NetworkRequestExtraInfo>,
    body_chunks: mpsc::UnboundedReceiver<String>,
    cancel_handle: FetchCancelHandle,
    completion: Option<oneshot::Receiver<Result<()>>>,
}

impl StreamingHtmlResponse {
    pub fn new(
        final_url: Url,
        status: u16,
        headers: Vec<(String, String)>,
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

#[derive(Debug)]
pub struct StreamingRawResponse {
    pub final_url: Url,
    pub status: u16,
    pub status_text: Option<String>,
    pub headers: Vec<(String, String)>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirected: bool,
    pub redirect_chain: Vec<RedirectInfo>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<NegotiatedHttpVersion>,
    network_request_extra_info: Option<NetworkRequestExtraInfo>,
    body_chunks: mpsc::UnboundedReceiver<Vec<u8>>,
    cancel_handle: FetchCancelHandle,
    completion: Option<oneshot::Receiver<Result<()>>>,
    lifetime_lease: Option<StreamingResponseLifetimeLease>,
}

impl StreamingRawResponse {
    pub fn new(
        final_url: Url,
        status: u16,
        headers: Vec<(String, String)>,
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
            negotiated_http_version: None,
            network_request_extra_info: None,
            body_chunks,
            cancel_handle,
            completion: Some(completion),
            lifetime_lease: None,
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
            negotiated_http_version: head.negotiated_http_version,
            network_request_extra_info: None,
            body_chunks,
            cancel_handle,
            completion: Some(completion),
            lifetime_lease: None,
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

    pub async fn next_chunk(&mut self) -> Option<Vec<u8>> {
        self.body_chunks.recv().await
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
        self.body_chunks.try_recv().ok()
    }

    /// Returns whether the producer has closed the body channel and every
    /// published chunk has already been consumed.
    ///
    /// This is deliberately narrower than request completion: callers still
    /// use [`Self::finish`] to observe the terminal fetch result. It only lets
    /// a streaming consumer distinguish an open, temporarily empty body from
    /// an already exhausted one without waiting.
    pub fn body_chunk_stream_is_exhausted(&self) -> bool {
        self.body_chunks.is_closed() && self.body_chunks.is_empty()
    }

    pub async fn finish(&mut self) -> Result<()> {
        while self.body_chunks.recv().await.is_some() {}
        let completion = self
            .completion
            .as_mut()
            .expect("streaming raw completion should only be awaited once")
            .await;
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
        let network_request_extra_info = self.network_request_extra_info.clone();
        let (head, body) = self.into_body();
        RawResponse::from_head_and_body_source(head, body)
            .await
            .map(|response| response.with_network_request_extra_info(network_request_extra_info))
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
            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
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
}
