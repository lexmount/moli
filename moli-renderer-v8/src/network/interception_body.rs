use std::sync::Arc;

use anyhow::{Result, anyhow, ensure};
use moli_fetch::{FetchCancelHandle, StreamingRawResponse};
use tokio::sync::Mutex;

use crate::types::{SubresourceResponseBody, SubresourceResponseBodyWriter};

/// A pending response body, independent of any Page, V8 context or decision
/// port. Reads start on demand and share the existing captured-body disk pool.
/// A fail/fulfill decision cancels the transfer without waiting for a reader.
#[derive(Clone, Debug)]
pub struct ResourceInterceptionBody {
    state: Arc<Mutex<BodyState>>,
    captured: Arc<parking_lot::Mutex<BodyCapture>>,
    cancel: FetchCancelHandle,
}

impl PartialEq for ResourceInterceptionBody {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl Eq for ResourceInterceptionBody {}

#[derive(Debug)]
enum BodyState {
    Receiving(Box<StreamingRawResponse>),
    Complete,
    Failed(String),
}

/// Capture is separately accessible without waiting for a transport read.
/// Cancelling a Page consumer can seal its delivered prefix immediately.
#[derive(Debug)]
enum BodyCapture {
    Receiving(SubresourceResponseBodyWriter),
    Complete(SubresourceResponseBody),
}

impl BodyCapture {
    fn len(&self) -> usize {
        match self {
            Self::Receiving(writer) => writer.len(),
            Self::Complete(body) => body.len(),
        }
    }

    fn read_chunk(&self, offset: usize, size: usize) -> std::io::Result<Vec<u8>> {
        match self {
            Self::Receiving(writer) => writer.read_chunk(offset, size),
            Self::Complete(body) => body.read_chunk(offset, size),
        }
    }

    fn finish(&mut self) -> SubresourceResponseBody {
        if let Self::Receiving(writer) = self {
            *self = Self::Complete(std::mem::take(writer).finish());
        }
        let Self::Complete(body) = self else {
            unreachable!("capture was sealed above")
        };
        body.clone()
    }
}

impl ResourceInterceptionBody {
    pub(crate) fn new(
        response: StreamingRawResponse,
        disk_pool: Option<moli_disk_pool::DiskPool>,
    ) -> Self {
        Self {
            cancel: response.cancellation_handle(),
            state: Arc::new(Mutex::new(BodyState::Receiving(Box::new(response)))),
            captured: Arc::new(parking_lot::Mutex::new(BodyCapture::Receiving(
                SubresourceResponseBodyWriter::with_disk_pool(disk_pool),
            ))),
        }
    }

    pub(crate) fn from_complete(body: SubresourceResponseBody) -> Self {
        Self {
            cancel: FetchCancelHandle::new(),
            state: Arc::new(Mutex::new(BodyState::Complete)),
            captured: Arc::new(parking_lot::Mutex::new(BodyCapture::Complete(body))),
        }
    }

    /// Retire this transfer, including a read currently waiting for body data.
    /// This does not consume or resume the interception decision port.
    pub fn cancellation_handle(&self) -> FetchCancelHandle {
        self.cancel.clone()
    }

    /// Move an unread transport directly to its terminal consumer. If a Fetch
    /// client already started reading, continue through the shared capture.
    pub async fn take_unread_response(&self) -> Option<StreamingRawResponse> {
        let mut state = self.state.lock().await;
        if self.captured.lock().len() != 0 || !matches!(*state, BodyState::Receiving(_)) {
            return None;
        }
        let BodyState::Receiving(response) = std::mem::replace(
            &mut *state,
            BodyState::Failed("response body transferred to download".into()),
        ) else {
            unreachable!()
        };
        Some(*response)
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
        if let Ok(mut state) = self.state.try_lock()
            && matches!(*state, BodyState::Receiving(_))
        {
            // Retained protocol handles must not keep a retired transport or
            // its runtime lease alive. An active reader takes this same path
            // through receive() when cancellation wins its wait.
            *state = BodyState::Failed("net::ERR_ABORTED".into());
        }
        self.captured.lock().finish();
    }

    /// Share the existing capture, bounded by bytes accepted by the consumer.
    /// This remains synchronous even if a protocol reader is waiting for EOF.
    pub(crate) fn finish_capture(&self, delivered: usize) -> SubresourceResponseBody {
        self.cancel();
        self.captured.lock().finish().prefix(delivered)
    }

    pub async fn capture(&self, limit: usize) -> Result<SubresourceResponseBody> {
        let mut state = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(anyhow!("net::ERR_ABORTED")),
            state = self.state.lock() => state,
        };
        loop {
            match &*state {
                BodyState::Complete => {
                    let body = self.captured.lock().finish();
                    ensure!(
                        body.len() <= limit,
                        "response body exceeds materialization limit"
                    );
                    return Ok(body);
                }
                BodyState::Failed(error) => return Err(anyhow!(error.clone())),
                BodyState::Receiving(_) => {
                    ensure!(
                        self.captured.lock().len() <= limit,
                        "response body exceeds materialization limit"
                    );
                }
            }
            let _ = self.receive(&mut state).await?;
        }
    }

    /// Returns available bytes without waiting for EOF. Only a read beyond the
    /// currently captured prefix waits for the next body chunk or terminal.
    pub async fn read_chunk(&self, offset: usize, size: usize) -> Result<(Vec<u8>, bool)> {
        let mut state = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(anyhow!("net::ERR_ABORTED")),
            state = self.state.lock() => state,
        };
        loop {
            match &*state {
                BodyState::Complete => {
                    let body = self.captured.lock().finish();
                    let bytes = body.read_chunk(offset, size)?;
                    let eof = offset.saturating_add(bytes.len()) >= body.len();
                    return Ok((bytes, eof));
                }
                BodyState::Failed(error) => return Err(anyhow!(error.clone())),
                BodyState::Receiving(_) => {
                    let available = {
                        let captured = self.captured.lock();
                        (offset < captured.len() || size == 0)
                            .then(|| captured.read_chunk(offset, size))
                    };
                    if let Some(bytes) = available {
                        return Ok((bytes?, false));
                    }
                    let start = self.captured.lock().len();
                    if let Some(bytes) = self.receive(&mut state).await?
                        && offset == start
                        && bytes.len() <= size
                    {
                        // The ordinary async XHR consumer can forward the
                        // original chunk instead of reading it back from disk.
                        return Ok((bytes, false));
                    }
                }
            }
        }
    }

    async fn receive(&self, state: &mut BodyState) -> Result<Option<Vec<u8>>> {
        let BodyState::Receiving(response) = state else {
            return Ok(None);
        };
        let next = tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(anyhow!("net::ERR_ABORTED")),
            next = async {
                if let Some(bytes) = response.next_chunk().await {
                    return Ok(Some(bytes));
                }
                response.finish().await?;
                Ok(None)
            } => next,
        };
        match next {
            Ok(Some(bytes)) => {
                let mut captured = self.captured.lock();
                let BodyCapture::Receiving(writer) = &mut *captured else {
                    *state = BodyState::Failed("net::ERR_ABORTED".into());
                    return Err(anyhow!("net::ERR_ABORTED"));
                };
                writer.append(&bytes);
                return Ok(Some(bytes));
            }
            Ok(None) => {
                self.captured.lock().finish();
                *state = BodyState::Complete;
            }
            Err(error) => {
                self.captured.lock().finish();
                *state = BodyState::Failed(format!("{error:#}"));
                return Err(error);
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn download_takes_only_unread_transports_and_keeps_shared_cancellation() {
        for read_prefix in [false, true] {
            let (chunks, body_rx) = tokio::sync::mpsc::unbounded_channel();
            let (complete, completion_rx) = tokio::sync::oneshot::channel();
            let cancel = FetchCancelHandle::new();
            let response = StreamingRawResponse::new(
                "https://capture.test/download".parse().unwrap(),
                200,
                Vec::new(),
                None,
                Vec::new(),
                false,
                Vec::new(),
                body_rx,
                cancel.clone(),
                completion_rx,
            );
            let body = ResourceInterceptionBody::new(response, None);
            chunks.send(b"first".to_vec()).unwrap();
            if read_prefix {
                assert_eq!(body.read_chunk(0, 5).await.unwrap().0, b"first");
                assert!(body.take_unread_response().await.is_none());
                chunks.send(b"second".to_vec()).unwrap();
                drop(chunks);
                complete.send(Ok(())).unwrap();
                assert_eq!(
                    body.capture(usize::MAX)
                        .await
                        .unwrap()
                        .materialize_bytes()
                        .unwrap(),
                    b"firstsecond"
                );
            } else {
                let mut response = body.take_unread_response().await.unwrap();
                assert!(body.take_unread_response().await.is_none());
                assert_eq!(response.next_chunk().await.unwrap(), b"first");
                body.cancellation_handle().cancel();
                assert!(cancel.is_cancelled());
            }
        }
    }

    #[tokio::test]
    async fn cancelling_page_capture_does_not_wait_for_a_protocol_reader() {
        let (chunks, body_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_complete, completion_rx) = tokio::sync::oneshot::channel();
        let response = StreamingRawResponse::new(
            "https://capture.test/body".parse().unwrap(),
            200,
            Vec::new(),
            None,
            Vec::new(),
            false,
            Vec::new(),
            body_rx,
            FetchCancelHandle::new(),
            completion_rx,
        );
        let body = ResourceInterceptionBody::new(response, None);
        chunks.send(b"first".to_vec()).unwrap();
        assert_eq!(body.read_chunk(0, 5).await.unwrap().0, b"first");
        let protocol_read = body.capture(usize::MAX);
        tokio::pin!(protocol_read);
        assert!(futures_util::poll!(&mut protocol_read).is_pending());
        let partial = body.finish_capture(3);
        assert_eq!(partial.materialize_bytes().unwrap(), b"fir");
        assert!(protocol_read.await.is_err());
        assert_eq!(
            body.finish_capture(5).materialize_bytes().unwrap(),
            b"first"
        );
    }

    #[tokio::test]
    async fn transport_failure_retains_the_shared_capture() {
        let (chunks, body_rx) = tokio::sync::mpsc::unbounded_channel();
        let (complete, completion_rx) = tokio::sync::oneshot::channel();
        let response = StreamingRawResponse::new(
            "https://capture.test/body".parse().unwrap(),
            200,
            Vec::new(),
            None,
            Vec::new(),
            false,
            Vec::new(),
            body_rx,
            FetchCancelHandle::new(),
            completion_rx,
        );
        let body = ResourceInterceptionBody::new(response, None);
        chunks.send(b"partial".to_vec()).unwrap();
        drop(chunks);
        complete.send(Err(anyhow!("transport failed"))).unwrap();
        assert_eq!(body.read_chunk(0, 7).await.unwrap().0, b"partial");
        assert!(body.read_chunk(7, 1).await.is_err());
        assert_eq!(
            body.finish_capture(7).materialize_bytes().unwrap(),
            b"partial"
        );
    }
}
